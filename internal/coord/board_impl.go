package coord

import (
	"context"
	"fmt"
	"slices"
	"sort"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/store"
)

// The Board's implementation (board.go has the contract).

const (
	maxNoticeField = 600 // runes of objective / done criteria in an assigned notice
	sweeper        = "irori"
)

// refusalRec is a folded lease.refused row with its parsed resource.
type refusalRec struct {
	Refusal
	res Resource
}

// checkReason validates a block reason or unblock note (already trimmed).
func checkReason(s string, required bool) error {
	if required && s == "" {
		return fmt.Errorf("%w: a blocked task needs a reason", ErrInvalid)
	}
	if utf8.RuneCountInString(s) > MaxReason {
		return fmt.Errorf("%w: longer than %d characters", ErrInvalid, MaxReason)
	}
	return nil
}

// checkBlocker validates and normalizes what blocks task self ("" while it
// is being created). nil is fine. The caller holds c.mu.
func (c *coord) checkBlocker(bb *BlockedBy, self string) (*BlockedBy, error) {
	if bb == nil {
		return nil, nil
	}
	out := BlockedBy{Kind: bb.Kind, Ref: strings.TrimSpace(bb.Ref)}
	if utf8.RuneCountInString(out.Ref) > MaxRef || strings.ContainsAny(out.Ref, "\r\n") {
		return nil, fmt.Errorf("%w: blocker ref longer than %d characters or on several lines", ErrInvalid, MaxRef)
	}
	switch out.Kind {
	case BlockedOnLease:
		n, err := NormalizeResource(out.Ref)
		if err != nil {
			return nil, fmt.Errorf("%w: blocked on a lease: %v", ErrInvalid, err)
		}
		// SweepNotices closes a lease blocker once nothing holds the lease,
		// so a lease nobody holds can't block (a port held by a process
		// outside Irori is BlockedOnExternal).
		if !c.leaseHeld(n, c.stamp()) {
			return nil, fmt.Errorf("%w: nobody leases %s; block on external or a person instead", ErrInvalid, n)
		}
		out.Ref = n
	case BlockedOnTask:
		if out.Ref == "" || out.Ref == self {
			return nil, fmt.Errorf("%w: blocked on a task needs another task's id", ErrInvalid)
		}
		o := c.tasks[out.Ref]
		if o == nil {
			return nil, fmt.Errorf("%w: blocked on task %q", ErrNotFound, out.Ref)
		}
		// SweepNotices closes a task blocker once that task is terminal, so
		// a done task can't block.
		if IsTerminal(o.State) {
			return nil, fmt.Errorf("%w: task %s is already %s", ErrInvalid, out.Ref, o.State)
		}
	case BlockedOnPerson:
		if out.Ref != "" && !c.isOwner(out.Ref) {
			return nil, fmt.Errorf("%w: blocked on a person needs \"\" or a user: key", ErrInvalid)
		}
	case BlockedOnAgent:
		if out.Ref == "" || c.isOwner(out.Ref) {
			return nil, fmt.Errorf("%w: blocked on an agent needs its key", ErrInvalid)
		}
	case BlockedOnExternal:
	default:
		return nil, fmt.Errorf("%w: blocker kind %q (lease|task|person|agent|external)", ErrInvalid, bb.Kind)
	}
	return &out, nil
}

// anyTask finds a task in any state. The caller holds c.mu.
func (c *coord) anyTask(id string) (*Task, error) {
	if t := c.tasks[id]; t != nil {
		return t, nil
	}
	return nil, fmt.Errorf("%w: task %q", ErrNotFound, id)
}

// checkKey validates a key that becomes a holder (an assignee, a joiner):
// one line, at most MaxKey runes, and not Irori's own.
func checkKey(k string) error {
	if k == sweeper {
		return fmt.Errorf("%w: %q is Irori itself", ErrInvalid, k)
	}
	if utf8.RuneCountInString(k) > MaxKey || strings.IndexFunc(k, unicode.IsControl) >= 0 {
		return fmt.Errorf("%w: a key longer than %d characters or with control characters", ErrInvalid, MaxKey)
	}
	return nil
}

func callers(by string, more ...string) (string, []string, error) {
	by = strings.TrimSpace(by)
	if by == "" {
		return "", nil, fmt.Errorf("%w: no caller", ErrInvalid)
	}
	for i := range more {
		more[i] = strings.TrimSpace(more[i])
		if more[i] == "" {
			return "", nil, fmt.Errorf("%w: empty argument", ErrInvalid)
		}
	}
	return by, more, nil
}

// ---- assign, join, leave, share ----

func (c *coord) assignNotice(ctx context.Context, by, id, to string) (Task, Notice, error) {
	t, wrote, err := c.assign(ctx, by, id, to, true)
	if err != nil || !wrote {
		return t, Notice{}, err
	}
	by, to = strings.TrimSpace(by), strings.TrimSpace(to)
	n := Notice{Kind: NoticeAssigned, TaskID: t.ID, Ref: "task:" + t.ID, Title: t.Title, By: by}
	if !c.isOwner(to) && to != by {
		n.To = []string{to}
	}
	var b strings.Builder
	fmt.Fprintf(&b, "%s assigned you a task: %q (task:%s).", by, t.Title, t.ID)
	if t.Objective != "" {
		fmt.Fprintf(&b, "\nObjective: %s", cut(t.Objective, maxNoticeField))
	}
	if t.DoneCriteria != "" {
		fmt.Fprintf(&b, "\nDone when: %s", cut(t.DoneCriteria, maxNoticeField))
	}
	if contains(t.Assigned, to) {
		b.WriteString("\nClaim it to start.")
	}
	b.WriteString(deeper(t.ID))
	n.Text = b.String()
	return t, n, nil
}

func (c *coord) join(ctx context.Context, by, id, who string) (Task, error) {
	by, w, err := callers(by, who)
	if err != nil {
		return Task{}, err
	}
	who = w[0]
	if err := checkKey(who); err != nil {
		return Task{}, err
	}
	if by != who && !c.isOwner(by) {
		return Task{}, fmt.Errorf("%w: only %s or a person may add %s", ErrNotAllowed, who, who)
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, err
	}
	if !t.Shared {
		return Task{}, fmt.Errorf("%w: task %s isn't shared; claim it instead", ErrNotAllowed, id)
	}
	if engaged(t, who) && t.State != Submitted {
		return copyTask(t), nil
	}
	if err := c.write(ctx, by, KindTaskJoined, taskMemberRow{TaskID: id, Who: who, By: by, At: c.stamp()}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
}

func (c *coord) leave(ctx context.Context, by, id, who string) (Task, error) {
	by, w, err := callers(by, who)
	if err != nil {
		return Task{}, err
	}
	who = w[0]
	if by != who && !c.isOwner(by) {
		return Task{}, fmt.Errorf("%w: only %s or a person may take %s off", ErrNotAllowed, who, who)
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, err
	}
	if !isHolder(t, who) {
		return Task{}, fmt.Errorf("%w: %s isn't on task %s", ErrNotFound, who, id)
	}
	if err := c.write(ctx, by, KindTaskLeft, taskMemberRow{TaskID: id, Who: who, By: by, At: c.stamp()}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
}

func (c *coord) setShared(ctx context.Context, by, id string, shared bool) (Task, error) {
	by, _, err := callers(by)
	if err != nil {
		return Task{}, err
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, err
	}
	if !c.mayManage(t, by) {
		return Task{}, fmt.Errorf("%w: share task %s", ErrNotAllowed, id)
	}
	if t.Shared == shared {
		return copyTask(t), nil
	}
	if !shared && len(t.Holders) > 1 {
		return Task{}, fmt.Errorf("%w: %d hold task %s; they leave first", ErrInvalid, len(t.Holders), id)
	}
	if err := c.write(ctx, by, KindTaskShared, taskSharedRow{TaskID: id, Shared: shared, By: by, At: c.stamp()}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
}

// ---- block, unblock ----

func (c *coord) block(ctx context.Context, by, id, reason string, bb *BlockedBy) (Task, error) {
	by, _, err := callers(by)
	if err != nil {
		return Task{}, err
	}
	reason = strings.TrimSpace(reason)
	if err := checkReason(reason, true); err != nil {
		return Task{}, err
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, err
	}
	if !c.mayManage(t, by) {
		return Task{}, fmt.Errorf("%w: block task %s", ErrNotAllowed, id)
	}
	// Block is Raise with the reason as the question (cards.go); a reason
	// longer than a question goes whole into the item's context.
	q, cx := blockQuestion(reason)
	out, _, err := c.raiseLocked(ctx, by, t, reason, q, cx, nil, bb, true, c.stamp())
	return out, err
}

func sameBlocker(a, b *BlockedBy) bool {
	if a == nil || b == nil {
		return a == b
	}
	return *a == *b
}

func (c *coord) unblock(ctx context.Context, by, id, note string) (Task, Notice, error) {
	by, _, err := callers(by)
	if err != nil {
		return Task{}, Notice{}, err
	}
	note = strings.TrimSpace(note)
	if err := checkReason(note, false); err != nil {
		return Task{}, Notice{}, err
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, Notice{}, err
	}
	if t.State != Blocked {
		return Task{}, Notice{}, fmt.Errorf("%w: task %s is %s, not blocked", ErrBadState, id, t.State)
	}
	n, err := c.closeBlocker(ctx, by, t, note, c.stamp())
	if err != nil {
		return Task{}, Notice{}, err
	}
	return copyTask(t), n, nil
}

// closeBlocker writes blocker.closed for blocked task t and builds the
// notice. The caller holds c.mu.
func (c *coord) closeBlocker(ctx context.Context, by string, t *Task, note string, now time.Time) (Notice, error) {
	reason := t.Reason
	restored := restoreState(t)
	if err := c.write(ctx, by, KindBlockerClosed, blockerClosedRow{
		TaskID: t.ID, By: by, Note: note, Restored: restored, At: now,
	}); err != nil {
		return Notice{}, err
	}
	n := Notice{Kind: NoticeUnblocked, TaskID: t.ID, Ref: "task:" + t.ID, Title: t.Title, By: by, To: c.recipients(t, by)}
	var b strings.Builder
	who := by
	if by == sweeper {
		who = "Irori"
	}
	fmt.Fprintf(&b, "%s unblocked %q (task:%s)", who, t.Title, t.ID)
	if note != "" {
		fmt.Fprintf(&b, ": %s", note)
	}
	fmt.Fprintf(&b, ". It was blocked on: %s. It is %s again.", cut(reason, maxNoticeField), restored)
	b.WriteString(deeper(t.ID))
	n.Text = b.String()
	return n, nil
}

// recipients are who hears about t: its agent holders other than by; with
// no holders, its creator if that is an agent other than by.
func (c *coord) recipients(t *Task, by string) []string {
	var to []string
	for _, h := range t.Holders {
		if h != by && !c.isOwner(h) && h != sweeper {
			to = append(to, h)
		}
	}
	if len(t.Holders) == 0 && t.CreatedBy != by && t.CreatedBy != "" && !c.isOwner(t.CreatedBy) && t.CreatedBy != sweeper {
		to = []string{t.CreatedBy}
	}
	return to
}

func deeper(id string) string {
	return "\nshow task:" + id + " for the card, its comments and links."
}

// cut trims s to n runes, marking the cut.
func cut(s string, n int) string {
	if utf8.RuneCountInString(s) <= n {
		return s
	}
	r := []rune(s)
	return string(r[:n-1]) + "…"
}

// ---- comments ----

func (c *coord) addComment(ctx context.Context, by, id, text string, wake bool, files []blob.FileRef) (Comment, Notice, error) {
	by, _, err := callers(by)
	if err != nil {
		return Comment{}, Notice{}, err
	}
	text = strings.TrimSpace(text)
	if text == "" {
		return Comment{}, Notice{}, fmt.Errorf("%w: an empty comment", ErrInvalid)
	}
	if utf8.RuneCountInString(text) > MaxComment {
		return Comment{}, Notice{}, fmt.Errorf("%w: comment longer than %d characters", ErrInvalid, MaxComment)
	}
	if len(files) > MaxAttachments {
		return Comment{}, Notice{}, fmt.Errorf("%w: at most %d attachments", ErrInvalid, MaxAttachments)
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.anyTask(id)
	if err != nil {
		return Comment{}, Notice{}, err
	}
	at := c.stamp()
	cid := newID(at)
	cm, err := c.writeComment(ctx, c.actorFor(by), taskCommentRow{
		TaskID: id, CommentID: cid, By: by, Text: text, Wake: wake, At: at, Attachments: files,
	})
	if err != nil {
		return Comment{}, Notice{}, err
	}
	n := Notice{
		Kind: NoticeComment, TaskID: id, Ref: "comment:" + cid, Title: t.Title, By: by,
		To: c.recipients(t, by), Wake: wake, Attachments: files,
		Text: fmt.Sprintf("%s commented on %q (task:%s):\n%s%s", by, t.Title, id, text, deeper(id)),
	}
	return cm, n, nil
}

// writeComment appends a task.comment row as actor and returns the comment
// it folded to. The caller holds c.mu.
func (c *coord) writeComment(ctx context.Context, actor string, row taskCommentRow) (Comment, error) {
	if err := c.writeAs(ctx, actor, KindTaskComment, row); err != nil {
		return Comment{}, err
	}
	cm := c.comments[row.CommentID]
	if cm == nil {
		return Comment{}, fmt.Errorf("%w: comment %s did not fold", ErrInvalid, row.CommentID)
	}
	return *cm, nil
}

// recordMessage puts an inbox message on its card's thread (contract in
// board.go). The writer rule is checked first: a row this device may not
// write would be ignored by its own fold.
func (c *coord) recordMessage(ctx context.Context, m MessageEntry) (Comment, error) {
	from, more, err := callers(m.From, m.TaskID)
	if err != nil {
		return Comment{}, err
	}
	id, to := more[0], strings.TrimSpace(m.To)
	if !strings.HasPrefix(m.MsgID, "msg_") || len(m.MsgID) == len("msg_") {
		return Comment{}, fmt.Errorf("%w: message id %q", ErrInvalid, m.MsgID)
	}
	text := strings.TrimSpace(m.Text)
	if m.Held {
		text = "" // held by the wake policy: naming a card doesn't get it past
	} else if utf8.RuneCountInString(text) > MaxComment {
		full := "… (full: msg:" + m.MsgID + ")"
		text = string([]rune(text)[:MaxComment-utf8.RuneCountInString(full)]) + full
	}
	actor := c.actorFor(from)
	if m.Writer == sweeper {
		actor = sweeper
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.anyTask(id)
	if err != nil {
		return Comment{}, err
	}
	if cm := c.comments[m.MsgID]; cm != nil {
		// Already there: a retry, the sweep, or another device. Only what
		// the entry lacks is written (the fold merges either order).
		if m.Gone && cm.Gone || !m.Gone && cm.primary {
			return *cm, nil
		}
	}
	if !mayRecord(from, to, m.Gone, c.st.DeviceID()) {
		return Comment{}, fmt.Errorf("%w: %s's message is recorded from its own device", ErrNotAllowed, from)
	}
	return c.writeComment(ctx, actor, taskCommentRow{
		TaskID: t.ID, CommentID: m.MsgID, MsgID: m.MsgID, By: from, To: to, Text: text,
		Files: max(m.Files, 0), Held: m.Held, Gone: m.Gone, At: c.stamp(),
	})
}

// mergeComment folds a row for a comment id already folded. Only a
// mirrored message has a second row: a gone row ORs Gone in, and the
// primary (non-gone) row, whichever came first, gives the entry its
// position, time and text, so the fold doesn't depend on arrival order.
// Any other duplicate is dropped. The caller holds c.mu.
func (c *coord) mergeComment(cm *Comment, p taskCommentRow, ev store.Event, at func(time.Time) time.Time) {
	if cm.MsgID == "" || p.MsgID != cm.MsgID || p.TaskID != cm.TaskID {
		return
	}
	changed := false
	if p.Gone && !cm.Gone {
		cm.Gone, changed = true, true
	}
	if !p.Gone && !cm.primary {
		cm.By, cm.To, cm.Text, cm.Wake, cm.At = p.By, p.To, p.Text, p.Wake, at(p.At)
		cm.Held, cm.Files, cm.Attachments = p.Held, p.Files, p.Attachments
		cm.Device, cm.HLC, cm.Seq, cm.primary = ev.DeviceID, ev.HLC, ev.Seq, true
		if t := c.tasks[cm.TaskID]; t != nil {
			t.UpdatedAt = cm.At
		}
		changed = true
	}
	if changed {
		c.note(Change{Kind: CommentAdded, TaskID: cm.TaskID})
	}
}

func (c *coord) commentAttachment(sha string) (blob.FileRef, bool) {
	c.mu.Lock()
	defer c.unlock()
	for _, cm := range c.comments {
		for _, f := range cm.Attachments {
			if f.SHA256 == sha {
				return f, true
			}
		}
	}
	return blob.FileRef{}, false
}

func (c *coord) listComments(taskID string) []Comment {
	c.mu.Lock()
	defer c.unlock()
	if c.tasks[taskID] == nil {
		return nil
	}
	out := []Comment{}
	for _, id := range c.byTask[taskID] {
		out = append(out, *c.comments[id])
	}
	return out
}

func (c *coord) getComment(id string) (Comment, bool) {
	c.mu.Lock()
	defer c.unlock()
	if cm := c.comments[id]; cm != nil {
		return *cm, true
	}
	return Comment{}, false
}

// ---- links ----

func (c *coord) addLink(ctx context.Context, by, id, link string) (Task, error) {
	by, _, err := callers(by)
	if err != nil {
		return Task{}, err
	}
	l, err := ParseLink(link)
	if err != nil {
		return Task{}, err
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.anyTask(id)
	if err != nil {
		return Task{}, err
	}
	if linkIndex(t.Links, l.Ref) >= 0 {
		return copyTask(t), nil
	}
	if len(t.Links) >= MaxLinks {
		return Task{}, fmt.Errorf("%w: task %s already has %d links", ErrInvalid, id, MaxLinks)
	}
	if err := c.write(ctx, by, KindTaskLinked, taskLinkedRow{TaskID: id, Link: l, By: by, At: c.stamp()}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
}

func (c *coord) removeLink(ctx context.Context, by, id, ref string) (Task, error) {
	by, _, err := callers(by)
	if err != nil {
		return Task{}, err
	}
	ref = strings.TrimSpace(ref)
	c.mu.Lock()
	defer c.unlock()
	t, err := c.anyTask(id)
	if err != nil {
		return Task{}, err
	}
	if !c.mayManage(t, by) {
		return Task{}, fmt.Errorf("%w: remove a link from task %s", ErrNotAllowed, id)
	}
	i := linkIndex(t.Links, ref)
	if i < 0 && ref != "" {
		// Accept the ref as typed before ParseLink normalized it.
		if l, err := ParseLink(ref); err == nil {
			i = linkIndex(t.Links, l.Ref)
		}
	}
	if i < 0 {
		return Task{}, fmt.Errorf("%w: task %s has no link %q", ErrNotFound, id, ref)
	}
	if err := c.write(ctx, by, KindTaskLinked, taskLinkedRow{TaskID: id, Link: t.Links[i], Removed: true, By: by, At: c.stamp()}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
}

// ---- refusals ----

// refusalActive: r.HeldBy still holds a lease that conflicts with r's
// resource and that was taken by the time of the refusal (a renewal keeps
// AcquiredAt). by can only take the resource once those leases are gone, so
// this is also "by hasn't since acquired it", and an inactive refusal never
// comes back. The caller holds c.mu.
func (c *coord) refusalActive(r *refusalRec, now time.Time) bool {
	for _, l := range c.held {
		if l.Holder == r.HeldBy && l.Until.After(now) && !l.AcquiredAt.After(r.At) && conflicts(l.res, r.res) {
			return true
		}
	}
	return false
}

// gcRefusals forgets refusals that are no longer active. The caller holds
// c.mu (or is replaying).
func (c *coord) gcRefusals(now time.Time) {
	keep := c.refusals[:0]
	for _, r := range c.refusals {
		if c.refusalActive(r, now) {
			keep = append(keep, r)
		}
	}
	clear(c.refusals[len(keep):])
	c.refusals = keep
}

// refuse logs a refused Acquire: one row per holder in the way, skipping
// any (resource, by, heldBy) with an active refusal. The caller holds c.mu.
func (c *coord) refuse(ctx context.Context, by string, res Resource, victims []*leaseRec, now time.Time) error {
	done := map[string]bool{}
	for _, v := range victims {
		if done[v.Holder] {
			continue
		}
		done[v.Holder] = true
		dup := false
		for _, r := range c.refusals {
			if r.Resource == res.Name && r.By == by && r.HeldBy == v.Holder && c.refusalActive(r, now) {
				dup = true
				break
			}
		}
		if dup {
			continue
		}
		if err := c.write(ctx, by, KindLeaseRefused, Refusal{
			Resource: res.Name, By: by, HeldBy: v.Holder, Held: v.Resource, Until: v.Until, At: now,
		}); err != nil {
			return err
		}
	}
	return nil
}

func (c *coord) activeRefusals() []Refusal {
	c.mu.Lock()
	defer c.unlock()
	now := c.stamp()
	c.prune(now)
	return c.refusalsLocked()
}

// refusalsLocked lists active refusals newest first. The caller holds c.mu
// and has pruned.
func (c *coord) refusalsLocked() []Refusal {
	out := []Refusal{}
	for i := len(c.refusals) - 1; i >= 0; i-- {
		out = append(out, c.refusals[i].Refusal)
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].At.After(out[j].At) })
	return out
}

// ---- board ----

// leaseHeld reports whether an unexpired lease conflicts with resource
// name. The caller holds c.mu.
func (c *coord) leaseHeld(name string, now time.Time) bool {
	res, err := ParseResource(name)
	if err != nil {
		res = Resource{Kind: ResourceNamed, Name: name}
	}
	for _, l := range c.held {
		if l.Until.After(now) && conflicts(l.res, res) {
			return true
		}
	}
	return false
}

// resolved reports why a blocked task's blocker went away by itself ("" if
// it hasn't). The caller holds c.mu.
func (c *coord) resolved(t *Task, now time.Time) string {
	if _, why := c.goneAway(t, now); len(why) > 0 {
		return why[0]
	}
	return ""
}

// goneAway lists the open items of a blocked task whose lease or task went
// away, with why for each. The caller holds c.mu.
func (c *coord) goneAway(t *Task, now time.Time) ([]string, []string) {
	if t.State != Blocked {
		return nil, nil
	}
	var ids, why []string
	for _, b := range t.Blockers {
		if !b.Open() || b.On == nil {
			continue
		}
		switch b.On.Kind {
		case BlockedOnLease:
			if !c.leaseHeld(b.On.Ref, now) {
				ids, why = append(ids, b.ID), append(why, "lease released")
			}
		case BlockedOnTask:
			if o := c.tasks[b.On.Ref]; o != nil && IsTerminal(o.State) {
				ids, why = append(ids, b.ID), append(why, "task "+string(o.State))
			}
		}
	}
	return ids, why
}

// overlapsOf lists pairs of a shared task's holders' leases that overlap.
// The caller holds c.mu.
func (c *coord) overlapsOf(t *Task, now time.Time) []string {
	if !t.Shared || len(t.Holders) < 2 {
		return nil
	}
	var mine []*leaseRec
	for _, l := range c.held {
		if l.Until.After(now) && isHolder(t, l.Holder) {
			mine = append(mine, l)
		}
	}
	sort.Slice(mine, func(i, j int) bool { return mine[i].Resource < mine[j].Resource })
	var out []string
	for i, a := range mine {
		for _, b := range mine[i+1:] {
			if a.Holder != b.Holder && conflicts(a.res, b.res) {
				out = append(out, a.Resource+" ~ "+b.Resource)
			}
		}
	}
	return out
}

func (c *coord) board(f BoardFilter) BoardView {
	c.mu.Lock()
	defer c.unlock()
	now := c.stamp()
	c.prune(now)
	v := BoardView{NeedsYou: []Card{}, Blocked: []Card{}, Working: []Card{}, Open: []Card{}, Done: []Card{}, Leases: []LeaseCard{}}
	limit := f.DoneLimit
	if limit <= 0 {
		limit = DefaultDoneCard
	}
	limit = min(limit, MaxDoneCard)
	viewer := strings.TrimSpace(f.For)
	agentViewer := viewer != "" && !c.isOwner(viewer)

	blocks := map[*leaseRec][]string{}
	for _, t := range c.tasks {
		if f.RepoKey != "" && t.RepoKey != f.RepoKey {
			continue
		}
		card := Card{Task: copyTask(t), Overlaps: c.overlapsOf(t, now)}
		if ids := c.byTask[t.ID]; len(ids) > 0 {
			last := *c.comments[ids[len(ids)-1]]
			card.LastComment = &last
		}
		if ids, _ := c.goneAway(t, now); len(ids) > 0 {
			if len(ids) == countOpen(t) {
				card.Resolved = true
			} else {
				card.Gone = ids
			}
		}
		for _, bl := range t.Blockers {
			if t.State != Blocked || !bl.Open() || bl.On == nil || bl.On.Kind != BlockedOnLease {
				continue
			}
			res, err := ParseResource(bl.On.Ref)
			if err == nil {
				for _, l := range c.held {
					if l.Until.After(now) && conflicts(l.res, res) && !contains(blocks[l], t.ID) {
						blocks[l] = append(blocks[l], t.ID)
					}
				}
			}
		}
		switch {
		case IsTerminal(t.State):
			v.Done = append(v.Done, card)
		case c.needsYou(t, viewer, agentViewer):
			v.NeedsYou = append(v.NeedsYou, card)
		case t.State == Blocked:
			v.Blocked = append(v.Blocked, card)
		case len(t.Holders) > 0:
			v.Working = append(v.Working, card)
		default:
			v.Open = append(v.Open, card)
		}
	}
	for _, g := range []*[]Card{&v.NeedsYou, &v.Blocked, &v.Working, &v.Open, &v.Done} {
		cs := *g
		sort.Slice(cs, func(i, j int) bool {
			if !cs[i].UpdatedAt.Equal(cs[j].UpdatedAt) {
				return cs[i].UpdatedAt.After(cs[j].UpdatedAt)
			}
			return cs[i].ID > cs[j].ID
		})
	}
	v.DoneTotal = len(v.Done)
	if len(v.Done) > limit {
		v.Done = v.Done[:limit:limit]
	}

	refused := map[*leaseRec][]Refusal{}
	for i := len(c.refusals) - 1; i >= 0; i-- {
		r := c.refusals[i]
		for _, l := range c.held {
			if l.Holder == r.HeldBy && l.Until.After(now) && !l.AcquiredAt.After(r.At) && conflicts(l.res, r.res) {
				refused[l] = append(refused[l], r.Refusal)
			}
		}
	}
	for _, l := range c.held {
		if !l.Until.After(now) || (len(blocks[l]) == 0 && len(refused[l]) == 0) {
			continue
		}
		// A refusal on another repo's paths stays off a filtered board
		// unless one of its tasks is blocked on it.
		if f.RepoKey != "" && len(blocks[l]) == 0 && l.res.Kind == ResourcePath && l.res.RepoKey != f.RepoKey {
			continue
		}
		lc := LeaseCard{Lease: l.Lease, Refused: refused[l], Blocks: blocks[l]}
		sort.Strings(lc.Blocks)
		sort.SliceStable(lc.Refused, func(i, j int) bool { return lc.Refused[i].At.After(lc.Refused[j].At) })
		v.Leases = append(v.Leases, lc)
	}
	sort.Slice(v.Leases, func(i, j int) bool {
		if !v.Leases[i].Until.Equal(v.Leases[j].Until) {
			return v.Leases[i].Until.Before(v.Leases[j].Until)
		}
		return v.Leases[i].Resource < v.Leases[j].Resource
	})
	return v
}

// needsYou picks the NeedsYou group (BoardView's rule 2).
// Any open blocker item counts, not only the first (Task.BlockedBy).
func (c *coord) needsYou(t *Task, viewer string, agentViewer bool) bool {
	var ons []*BlockedBy
	if t.State == Blocked {
		for _, b := range t.Blockers {
			if b.Open() && b.On != nil {
				ons = append(ons, b.On)
			}
		}
	}
	if agentViewer {
		if t.State != Blocked && isHolder(t, viewer) && !engaged(t, viewer) {
			return true
		}
		for _, bb := range ons {
			if bb.Kind == BlockedOnAgent && bb.Ref == viewer {
				return true
			}
		}
		return false
	}
	if t.State == InputRequired {
		return true
	}
	for _, bb := range ons {
		// The person is one person on every device: a blocker on any of
		// their owner keys needs them wherever they look (v0.4 §2).
		if bb.Kind == BlockedOnPerson && (viewer == "" || bb.Ref == "" || bb.Ref == viewer || c.isOwner(bb.Ref) && c.isOwner(viewer)) {
			return true
		}
	}
	if viewer == "" {
		return false
	}
	if c.isOwner(viewer) {
		return slices.ContainsFunc(t.Holders, c.isOwner)
	}
	return isHolder(t, viewer)
}

// ---- sweep ----

func (c *coord) sweepNotices(ctx context.Context) ([]Notice, error) {
	c.mu.Lock()
	defer c.unlock()
	now := c.stamp()
	if err := c.releaseGone(ctx, now); err != nil {
		return nil, err
	}
	var todo []*Task
	for _, t := range c.tasks {
		if c.resolved(t, now) != "" {
			todo = append(todo, t)
		}
	}
	sort.Slice(todo, func(i, j int) bool { return todo[i].ID < todo[j].ID })
	var out []Notice
	for _, t := range todo {
		ids, why := c.goneAway(t, now)
		if len(ids) == countOpen(t) {
			// Everything it waited on went away: unblock it.
			n, err := c.closeBlocker(ctx, sweeper, t, why[0], now)
			if err != nil {
				return out, err
			}
			out = append(out, n)
			continue
		}
		// Other items still wait (a question for the person): close only
		// these, and the task stays blocked.
		var raisers []string
		for _, b := range t.Blockers {
			if slices.Contains(ids, b.ID) {
				raisers = append(raisers, b.RaisedBy)
			}
		}
		for i, id := range ids {
			if err := c.write(ctx, sweeper, KindBlockerResolved, blockerResolvedRow{
				TaskID: t.ID, BlockerID: id, Answer: why[i], Option: -1, By: sweeper, Closed: true, At: now,
			}); err != nil {
				return out, err
			}
		}
		// Its raisers and holders hear, so a freed lease isn't missed.
		n := Notice{Kind: NoticeAnswered, TaskID: t.ID, Ref: "task:" + t.ID, Title: t.Title, By: sweeper}
		for _, r := range append(raisers, c.recipients(t, sweeper)...) {
			if r != "" && !c.isOwner(r) && r != sweeper && !slices.Contains(n.To, r) {
				n.To = append(n.To, r)
			}
		}
		var items []string
		for i, id := range ids {
			items = append(items, id+" ("+why[i]+")")
		}
		n.Text = fmt.Sprintf("Irori closed %s on %q (task:%s): what it waited on went away. %d more blockers are still open; board_unblock with blocker_id answers one.%s",
			strings.Join(items, ", "), t.Title, t.ID, countOpen(t), deeper(t.ID))
		if len(n.To) > 0 {
			out = append(out, n)
		}
	}
	// Watchers hear what changed on their cards since the last sweep
	// (cards.go), including the closes above.
	return append(out, c.watchNotices()...), nil
}

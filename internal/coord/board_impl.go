package coord

import (
	"context"
	"fmt"
	"sort"
	"strings"
	"time"
	"unicode/utf8"
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
		if c.tasks[out.Ref] == nil {
			return nil, fmt.Errorf("%w: blocked on task %q", ErrNotFound, out.Ref)
		}
	case BlockedOnPerson:
		if out.Ref != "" && !IsPerson(out.Ref) {
			return nil, fmt.Errorf("%w: blocked on a person needs \"\" or a user: key", ErrInvalid)
		}
	case BlockedOnAgent:
		if out.Ref == "" || IsPerson(out.Ref) {
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
	if !IsPerson(to) && to != by {
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
	if t.State == Submitted {
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
	if by != who && !IsPerson(by) {
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
	if isHolder(t, who) && t.State != Submitted {
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
	if by != who && !IsPerson(by) {
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
	if !mayManage(t, by) {
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
	if !mayManage(t, by) {
		return Task{}, fmt.Errorf("%w: block task %s", ErrNotAllowed, id)
	}
	nb, err := c.checkBlocker(bb, id)
	if err != nil {
		return Task{}, err
	}
	if t.State == Blocked && t.Reason == reason && sameBlocker(t.BlockedBy, nb) {
		return copyTask(t), nil
	}
	if err := c.write(ctx, by, KindTaskState, taskStateRow{
		TaskID: id, State: Blocked, Note: t.Note, Reason: reason, BlockedBy: nb, By: by, At: c.stamp(),
	}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
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
	n := Notice{Kind: NoticeUnblocked, TaskID: t.ID, Ref: "task:" + t.ID, Title: t.Title, By: by, To: recipients(t, by)}
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
func recipients(t *Task, by string) []string {
	var to []string
	for _, h := range t.Holders {
		if h != by && !IsPerson(h) && h != sweeper {
			to = append(to, h)
		}
	}
	if len(t.Holders) == 0 && t.CreatedBy != by && t.CreatedBy != "" && !IsPerson(t.CreatedBy) && t.CreatedBy != sweeper {
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

func (c *coord) addComment(ctx context.Context, by, id, text string, wake bool) (Comment, Notice, error) {
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
	c.mu.Lock()
	defer c.unlock()
	t, err := c.anyTask(id)
	if err != nil {
		return Comment{}, Notice{}, err
	}
	at := c.stamp()
	cid := newID(at)
	if err := c.write(ctx, by, KindTaskComment, taskCommentRow{
		TaskID: id, CommentID: cid, By: by, Text: text, Wake: wake, At: at,
	}); err != nil {
		return Comment{}, Notice{}, err
	}
	cm := *c.comments[cid]
	n := Notice{
		Kind: NoticeComment, TaskID: id, Ref: "comment:" + cid, Title: t.Title, By: by,
		To: recipients(t, by), Wake: wake,
		Text: fmt.Sprintf("%s commented on %q (task:%s):\n%s%s", by, t.Title, id, text, deeper(id)),
	}
	return cm, n, nil
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
	if !mayManage(t, by) {
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
	if t.State != Blocked || t.BlockedBy == nil {
		return ""
	}
	switch t.BlockedBy.Kind {
	case BlockedOnLease:
		if !c.leaseHeld(t.BlockedBy.Ref, now) {
			return "lease released"
		}
	case BlockedOnTask:
		if o := c.tasks[t.BlockedBy.Ref]; o != nil && IsTerminal(o.State) {
			return "task " + string(o.State)
		}
	}
	return ""
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
	agentViewer := viewer != "" && !IsPerson(viewer)

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
		card.Resolved = c.resolved(t, now) != ""
		if t.State == Blocked && t.BlockedBy != nil && t.BlockedBy.Kind == BlockedOnLease {
			res, err := ParseResource(t.BlockedBy.Ref)
			if err == nil {
				for _, l := range c.held {
					if l.Until.After(now) && conflicts(l.res, res) {
						blocks[l] = append(blocks[l], t.ID)
					}
				}
			}
		}
		switch {
		case IsTerminal(t.State):
			v.Done = append(v.Done, card)
		case needsYou(t, viewer, agentViewer):
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
func needsYou(t *Task, viewer string, agentViewer bool) bool {
	bb := t.BlockedBy
	if t.State != Blocked {
		bb = nil
	}
	if agentViewer {
		return t.State == Submitted && isHolder(t, viewer) ||
			bb != nil && bb.Kind == BlockedOnAgent && bb.Ref == viewer
	}
	switch {
	case t.State == InputRequired:
		return true
	case bb != nil && bb.Kind == BlockedOnPerson:
		return viewer == "" || bb.Ref == "" || bb.Ref == viewer
	}
	return viewer != "" && isHolder(t, viewer)
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
		n, err := c.closeBlocker(ctx, sweeper, t, c.resolved(t, now), now)
		if err != nil {
			return out, err
		}
		out = append(out, n)
	}
	return out, nil
}

package coord

import (
	"context"
	"encoding/json"
	"fmt"
	"net/url"
	"regexp"
	"slices"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/StephenSHorton/irori/internal/store"
)

// The cards' implementation (cards.go has the contract).

const (
	maxWatchLines   = 8  // lines in one watch digest
	maxWatchPending = 32 // queued watch lines per task; older ones are counted, not kept
)

// Row payloads.

// blockerRow is the item a blocked task.status row raises.
type blockerRow struct {
	ID       string   `json:"id"`
	Question string   `json:"question"`
	Context  string   `json:"context,omitempty"`
	Options  []Option `json:"options,omitempty"`
	// ReasonOnly marks the item Block makes from its reason: blocking
	// again through Block replaces it (Replace, same ID) instead of adding
	// one, as Block always did.
	ReasonOnly bool `json:"reasonOnly,omitempty"`
	Replace    bool `json:"replace,omitempty"`
	// On is what the item waits on (Blocker.On).
	On *BlockedBy `json:"on,omitempty"`
}

type blockerOptionRow struct {
	TaskID    string    `json:"taskId"`
	BlockerID string    `json:"blockerId"`
	Answer    string    `json:"answer"`
	Why       string    `json:"why,omitempty"`
	By        string    `json:"by"`
	At        time.Time `json:"at"`
}

type blockerResolvedRow struct {
	TaskID    string    `json:"taskId"`
	BlockerID string    `json:"blockerId"`
	Answer    string    `json:"answer"`
	Option    int       `json:"option"`
	By        string    `json:"by"`
	Restored  TaskState `json:"restored,omitempty"`
	// Closed: the sweeper closed the item because what it waited on went
	// away (Resolution.Closed); Answer is why.
	Closed bool      `json:"closed,omitempty"`
	At     time.Time `json:"at"`
}

type taskStepsRow struct {
	TaskID string    `json:"taskId"`
	Steps  []Step    `json:"steps"`
	By     string    `json:"by"`
	At     time.Time `json:"at"`
}

type taskStepRow struct {
	TaskID string    `json:"taskId"`
	StepID string    `json:"stepId"`
	Done   *bool     `json:"done,omitempty"`
	Doing  *bool     `json:"doing,omitempty"`
	By     string    `json:"by"`
	At     time.Time `json:"at"`
}

type taskWatchedRow struct {
	TaskID   string    `json:"taskId"`
	Who      string    `json:"who"`
	Watching bool      `json:"watching"`
	By       string    `json:"by"`
	At       time.Time `json:"at"`
}

// watchLine is one queued change on a watched card.
type watchLine struct {
	by, what string
	also     []string // told even when not watching (the raiser, for a recommendation)
}

type watchQueue struct {
	lines   []watchLine
	dropped int
}

// ---- validation ----

func checkText(what, s string, minRunes, maxRunes int) error {
	n := utf8.RuneCountInString(s)
	if n < minRunes {
		return fmt.Errorf("%w: %s is empty", ErrInvalid, what)
	}
	if n > maxRunes {
		return fmt.Errorf("%w: %s longer than %d characters", ErrInvalid, what, maxRunes)
	}
	return nil
}

// cleanOptions trims and checks a raise's options, setting By and At.
func cleanOptions(in []Option, by string, at time.Time) ([]Option, error) {
	if len(in) > MaxOptions {
		return nil, fmt.Errorf("%w: more than %d recommended answers", ErrInvalid, MaxOptions)
	}
	var out []Option
	for _, o := range in {
		a, why := strings.TrimSpace(o.Answer), strings.TrimSpace(o.Why)
		if err := checkText("a recommended answer", a, 1, MaxAnswer); err != nil {
			return nil, err
		}
		if err := checkText("a recommendation's why", why, 0, MaxWhy); err != nil {
			return nil, err
		}
		if optionIndex(out, a) >= 0 {
			continue
		}
		out = append(out, Option{Answer: a, Why: why, By: by, At: at})
	}
	return out, nil
}

func optionIndex(os []Option, answer string) int {
	for i, o := range os {
		if strings.EqualFold(o.Answer, answer) {
			return i
		}
	}
	return -1
}

// ---- ids ----

// seqOf reads the number of a short id ("b12" with prefix "b" is 12).
func seqOf(id, prefix string) int {
	n, err := strconv.Atoi(strings.TrimPrefix(id, prefix))
	if err != nil || !strings.HasPrefix(id, prefix) {
		return 0
	}
	return n
}

func (c *coord) nextBlockerID(taskID string) string {
	return "b" + strconv.Itoa(c.blockerSeq[taskID]+1)
}

// ---- fold helpers (the caller holds c.mu or is replaying) ----

func blockerIndex(t *Task, id string) int {
	for i, b := range t.Blockers {
		if b.ID == id {
			return i
		}
	}
	return -1
}

func openWithQuestion(t *Task, q string) int {
	for i, b := range t.Blockers {
		if b.Open() && b.Question == q {
			return i
		}
	}
	return -1
}

func countOpen(t *Task) int {
	n := 0
	for _, b := range t.Blockers {
		if b.Open() {
			n++
		}
	}
	return n
}

// syncReason sets Reason to the first open item's reason and BlockedBy to
// the first open item's On (nil when none has one, or none is open).
func syncReason(t *Task) {
	t.Reason, t.BlockedBy = "", nil
	for _, b := range t.Blockers {
		if !b.Open() {
			continue
		}
		if t.Reason == "" {
			t.Reason = b.reason
		}
		if t.BlockedBy == nil && b.On != nil {
			on := *b.On
			t.BlockedBy = &on
		}
	}
}

func copyOn(bb *BlockedBy) *BlockedBy {
	if bb == nil {
		return nil
	}
	b := *bb
	return &b
}

// closeOpen resolves every open item as closed, with note as the answer.
func closeOpen(t *Task, by, note string, at time.Time) {
	for i := range t.Blockers {
		if t.Blockers[i].Open() {
			t.Blockers[i].Resolved = &Resolution{Answer: note, Option: -1, By: by, At: at, Closed: true}
		}
	}
	trimResolved(t)
}

// trimResolved keeps the newest KeepResolved resolved items (by position:
// items are kept in the order raised).
func trimResolved(t *Task) {
	n := len(t.Blockers) - countOpen(t)
	if n <= KeepResolved {
		return
	}
	drop := n - KeepResolved
	keep := t.Blockers[:0:0]
	for _, b := range t.Blockers {
		if !b.Open() && drop > 0 {
			drop--
			continue
		}
		keep = append(keep, b)
	}
	t.Blockers = keep
}

// foldBlocked applies a task.status row with state blocked: it adds (or,
// for a reason-only row on a task already blocked, replaces) an item.
func (c *coord) foldBlocked(t *Task, p taskStateRow, wasBlocked bool, at time.Time) string {
	if p.Blocker != nil && p.Blocker.ID != "" {
		if i := blockerIndex(t, p.Blocker.ID); i >= 0 && p.Blocker.Replace && t.Blockers[i].Open() {
			b := &t.Blockers[i]
			b.Question, b.Context, b.reason, b.RaisedBy, b.At = p.Blocker.Question, p.Blocker.Context, cmpStr(p.Reason, p.Blocker.Question), p.By, at
			b.On = copyOn(p.Blocker.On)
			return "changed the blocker: " + b.Question
		}
		if blockerIndex(t, p.Blocker.ID) >= 0 {
			return ""
		}
		if i := openWithQuestion(t, p.Blocker.Question); i >= 0 {
			// The same question again: only what it waits on can change.
			t.Blockers[i].On = copyOn(p.Blocker.On)
			return ""
		}
		c.blockerSeq[t.ID] = max(c.blockerSeq[t.ID], seqOf(p.Blocker.ID, "b"))
		var opts []Option
		for _, o := range p.Blocker.Options {
			if len(opts) < MaxOptions && optionIndex(opts, o.Answer) < 0 {
				o.At = cmpOr(o.At, at)
				opts = append(opts, o)
			}
		}
		t.Blockers = append(t.Blockers, Blocker{
			ID: p.Blocker.ID, Question: p.Blocker.Question, Context: p.Blocker.Context,
			RaisedBy: p.By, At: at, Options: opts, reason: cmpStr(p.Reason, p.Blocker.Question),
			legacy: p.Blocker.ReasonOnly, On: copyOn(p.Blocker.On),
		})
		return "raised a blocker: " + p.Blocker.Question
	}
	// A reason-only row (written before items existed): re-blocking
	// replaced the reason, so it replaces the item it made.
	q, ctxText := p.Reason, ""
	if utf8.RuneCountInString(q) > MaxQuestion {
		q, ctxText = cut(p.Reason, MaxQuestion), p.Reason
	}
	if wasBlocked {
		for i := range t.Blockers {
			if b := &t.Blockers[i]; b.Open() && b.legacy {
				b.Question, b.Context, b.reason, b.RaisedBy, b.At = q, ctxText, p.Reason, p.By, at
				b.On = copyOn(p.BlockedBy)
				return "changed the blocker: " + q
			}
		}
	}
	c.blockerSeq[t.ID]++
	t.Blockers = append(t.Blockers, Blocker{
		ID: "b" + strconv.Itoa(c.blockerSeq[t.ID]), Question: q, Context: ctxText,
		RaisedBy: p.By, At: at, reason: p.Reason, legacy: true, On: copyOn(p.BlockedBy),
	})
	return "raised a blocker: " + q
}

func cmpStr(s, def string) string {
	if s == "" {
		return def
	}
	return s
}

func cmpOr(t, def time.Time) time.Time {
	if t.IsZero() {
		return def
	}
	return t
}

// normSteps keeps at most one Doing (the first) and no Done step Doing.
func normSteps(ss []Step) []Step {
	out := make([]Step, 0, len(ss))
	doing := false
	for _, s := range ss {
		if s.Done || doing {
			s.Doing = false
		}
		doing = doing || s.Doing
		out = append(out, s)
	}
	return out
}

// touch records activity on t and, outside replay, queues a watch line
// when watch is set.
func (c *coord) touch(t *Task, at time.Time, by, what string, watch bool, also ...string) {
	if what == "" {
		return
	}
	acts := append(c.activity[t.ID], Activity{At: at, By: by, What: what})
	if len(acts) > MaxActivity {
		acts = append(acts[:0:0], acts[len(acts)-MaxActivity:]...)
	}
	c.activity[t.ID] = acts
	if !watch || c.replaying {
		return
	}
	if len(t.Watchers) == 0 && len(also) == 0 {
		return
	}
	q := c.watchq[t.ID]
	if q == nil {
		q = &watchQueue{}
		c.watchq[t.ID] = q
	}
	q.lines = append(q.lines, watchLine{by: by, what: what, also: also})
	if len(q.lines) > maxWatchPending {
		q.dropped += len(q.lines) - maxWatchPending
		q.lines = append(q.lines[:0:0], q.lines[len(q.lines)-maxWatchPending:]...)
	}
}

// ---- raise, recommend, answer ----

func (c *coord) raise(ctx context.Context, by, id string, nb NewBlocker) (Task, Blocker, error) {
	by, _, err := callers(by)
	if err != nil {
		return Task{}, Blocker{}, err
	}
	q, cx := strings.TrimSpace(nb.Question), strings.TrimSpace(nb.Context)
	if err := checkText("the question", q, 1, MaxQuestion); err != nil {
		return Task{}, Blocker{}, err
	}
	if err := checkText("the context", cx, 0, MaxContext); err != nil {
		return Task{}, Blocker{}, err
	}
	c.mu.Lock()
	defer c.unlock()
	now := c.stamp()
	opts, err := cleanOptions(nb.Options, by, now)
	if err != nil {
		return Task{}, Blocker{}, err
	}
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, Blocker{}, err
	}
	if !mayManage(t, by) {
		return Task{}, Blocker{}, fmt.Errorf("%w: raise a blocker on task %s", ErrNotAllowed, id)
	}
	return c.raiseLocked(ctx, by, t, q, q, cx, opts, nb.BlockedBy, false, now)
}

// raiseLocked checks bb and writes the blocked row that raises an item.
// reasonOnly is Block: it replaces the open item an earlier Block made, and
// a nil bb stays nil; otherwise a nil bb means a person.
func (c *coord) raiseLocked(ctx context.Context, by string, t *Task, reason, q, cx string, opts []Option, bb *BlockedBy, reasonOnly bool, now time.Time) (Task, Blocker, error) {
	nb, err := c.checkBlocker(bb, t.ID)
	if err != nil {
		return Task{}, Blocker{}, err
	}
	if i := openWithQuestion(t, q); i >= 0 && t.State == Blocked && (nb == nil && !reasonOnly || sameBlocker(t.Blockers[i].On, nb)) {
		// The same question again, saying nothing new about what it waits on.
		return copyTask(t), copyBlocker(t.Blockers[i]), nil
	}
	if nb == nil && !reasonOnly {
		nb = &BlockedBy{Kind: BlockedOnPerson}
	}
	row := &blockerRow{ID: c.nextBlockerID(t.ID), Question: q, Context: cx, Options: opts, ReasonOnly: reasonOnly, On: nb}
	if reasonOnly && t.State == Blocked {
		for _, b := range t.Blockers {
			if b.Open() && b.legacy {
				row.ID, row.Replace = b.ID, true
				break
			}
		}
	}
	if !row.Replace && openWithQuestion(t, q) < 0 && countOpen(t) >= MaxOpenBlockers {
		return Task{}, Blocker{}, fmt.Errorf("%w: task %s already has %d open blockers", ErrInvalid, t.ID, MaxOpenBlockers)
	}
	bid := row.ID
	if err := c.write(ctx, by, KindTaskState, taskStateRow{
		TaskID: t.ID, State: Blocked, Note: t.Note, Reason: reason, BlockedBy: nb,
		Blocker: row, By: by, At: now,
	}); err != nil {
		return Task{}, Blocker{}, err
	}
	i := blockerIndex(t, bid)
	if i < 0 {
		i = openWithQuestion(t, q)
	}
	var out Blocker
	if i >= 0 {
		out = copyBlocker(t.Blockers[i])
	}
	return copyTask(t), out, nil
}

// mayRecommend: a holder, a watcher, the raiser, the creator or a person.
func mayRecommend(t *Task, b Blocker, by string) bool {
	return mayManage(t, by) || contains(t.Watchers, by) || b.RaisedBy == by
}

func (c *coord) recommend(ctx context.Context, by, id, bid, answer, why string) (Task, error) {
	by, _, err := callers(by)
	if err != nil {
		return Task{}, err
	}
	answer, why = strings.TrimSpace(answer), strings.TrimSpace(why)
	if err := checkText("the answer", answer, 1, MaxAnswer); err != nil {
		return Task{}, err
	}
	if err := checkText("the why", why, 0, MaxWhy); err != nil {
		return Task{}, err
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, err
	}
	i := blockerIndex(t, strings.TrimSpace(bid))
	if i < 0 {
		return Task{}, fmt.Errorf("%w: task %s has no blocker %q", ErrNotFound, id, bid)
	}
	b := t.Blockers[i]
	if !b.Open() {
		return Task{}, fmt.Errorf("%w: blocker %s is already resolved", ErrBadState, b.ID)
	}
	if !mayRecommend(t, b, by) {
		return Task{}, fmt.Errorf("%w: only its holders, watchers, the raiser, the creator or a person may recommend on task %s; watch it first", ErrNotAllowed, id)
	}
	if optionIndex(b.Options, answer) >= 0 {
		return copyTask(t), nil
	}
	if len(b.Options) >= MaxOptions {
		return Task{}, fmt.Errorf("%w: blocker %s already has %d recommended answers", ErrInvalid, b.ID, MaxOptions)
	}
	if err := c.write(ctx, by, KindBlockerOption, blockerOptionRow{
		TaskID: id, BlockerID: b.ID, Answer: answer, Why: why, By: by, At: c.stamp(),
	}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
}

func (c *coord) answer(ctx context.Context, by, id, bid string, a BlockerAnswer) (Task, Notice, error) {
	by, _, err := callers(by)
	if err != nil {
		return Task{}, Notice{}, err
	}
	text := strings.TrimSpace(a.Text)
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, Notice{}, err
	}
	i := blockerIndex(t, strings.TrimSpace(bid))
	if i < 0 {
		return Task{}, Notice{}, fmt.Errorf("%w: task %s has no blocker %q", ErrNotFound, id, bid)
	}
	b := t.Blockers[i]
	if !b.Open() {
		return Task{}, Notice{}, fmt.Errorf("%w: blocker %s is already resolved", ErrBadState, b.ID)
	}
	opt, recBy := -1, ""
	switch {
	case a.Pick < 0 || a.Pick > len(b.Options):
		return Task{}, Notice{}, fmt.Errorf("%w: blocker %s has %d recommended answers, not %d", ErrInvalid, b.ID, len(b.Options), a.Pick)
	case a.Pick > 0:
		opt = a.Pick - 1
		text, recBy = b.Options[opt].Answer, b.Options[opt].By
	default:
		if err := checkText("the answer", text, 1, MaxOwnAnswer); err != nil {
			return Task{}, Notice{}, err
		}
	}
	var restored TaskState
	if countOpen(t) == 1 {
		restored = restoreState(t)
	}
	if err := c.write(ctx, by, KindBlockerResolved, blockerResolvedRow{
		TaskID: id, BlockerID: b.ID, Answer: text, Option: opt, By: by, Restored: restored, At: c.stamp(),
	}); err != nil {
		return Task{}, Notice{}, err
	}
	n := Notice{Kind: NoticeAnswered, TaskID: id, Ref: "task:" + id, Title: t.Title, By: by, Wake: a.Wake}
	// The raiser hears first; the holders (who were blocked) hear too.
	if r := b.RaisedBy; r != by && r != "" && !IsPerson(r) && r != sweeper {
		n.To = []string{r}
	}
	for _, h := range recipients(t, by) {
		if !slices.Contains(n.To, h) {
			n.To = append(n.To, h)
		}
	}
	var sb strings.Builder
	fmt.Fprintf(&sb, "%s answered the blocker on %q (task:%s, blocker %s).\nQuestion: %s\nAnswer: %s",
		by, t.Title, id, b.ID, b.Question, cut(text, maxNoticeField))
	if recBy != "" {
		fmt.Fprintf(&sb, "\n(That is the answer %s recommended.)", recBy)
	}
	if restored != "" {
		fmt.Fprintf(&sb, "\nNo blockers remain: it is %s again.", t.State)
	} else {
		fmt.Fprintf(&sb, "\n%d more blockers are still open.", countOpen(t))
	}
	sb.WriteString(deeper(id))
	n.Text = sb.String()
	return copyTask(t), n, nil
}

// ---- steps ----

func (c *coord) setSteps(ctx context.Context, by, id string, in []Step) (Task, error) {
	by, _, err := callers(by)
	if err != nil {
		return Task{}, err
	}
	if len(in) > MaxSteps {
		return Task{}, fmt.Errorf("%w: more than %d steps", ErrInvalid, MaxSteps)
	}
	doing := 0
	for i := range in {
		in[i].Text = strings.TrimSpace(in[i].Text)
		in[i].ID = strings.TrimSpace(in[i].ID)
		if err := checkText("a step", in[i].Text, 1, MaxStepText); err != nil {
			return Task{}, err
		}
		if strings.ContainsAny(in[i].Text, "\r\n") {
			return Task{}, fmt.Errorf("%w: a step is one line", ErrInvalid)
		}
		if in[i].Done && in[i].Doing {
			return Task{}, fmt.Errorf("%w: step %q is both done and in progress", ErrInvalid, in[i].Text)
		}
		if in[i].Doing {
			doing++
		}
	}
	if doing > 1 {
		return Task{}, fmt.Errorf("%w: only one step can be in progress", ErrInvalid)
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, err
	}
	if !mayManage(t, by) {
		return Task{}, fmt.Errorf("%w: set the steps of task %s", ErrNotAllowed, id)
	}
	seq := c.stepSeq[id]
	used := map[string]bool{}
	out := make([]Step, 0, len(in))
	for _, s := range in {
		sid := ""
		if s.ID != "" && slices.ContainsFunc(t.Steps, func(o Step) bool { return o.ID == s.ID }) && !used[s.ID] {
			sid = s.ID
		}
		if sid == "" {
			for _, o := range t.Steps {
				if o.Text == s.Text && !used[o.ID] && !slices.ContainsFunc(in, func(x Step) bool { return x.ID == o.ID }) {
					sid = o.ID
					break
				}
			}
		}
		if sid == "" {
			seq++
			sid = "s" + strconv.Itoa(seq)
		}
		used[sid] = true
		out = append(out, Step{ID: sid, Text: s.Text, Done: s.Done, Doing: s.Doing})
	}
	if slices.Equal(out, t.Steps) || len(out) == 0 && len(t.Steps) == 0 {
		return copyTask(t), nil
	}
	if err := c.write(ctx, by, KindTaskSteps, taskStepsRow{TaskID: id, Steps: out, By: by, At: c.stamp()}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
}

func (c *coord) setStep(ctx context.Context, by, id, sid string, u StepUpdate) (Task, error) {
	by, _, err := callers(by)
	if err != nil {
		return Task{}, err
	}
	sid = strings.TrimSpace(sid)
	if u.Done == nil && u.Doing == nil {
		return Task{}, fmt.Errorf("%w: give done or doing", ErrInvalid)
	}
	if u.Done != nil && u.Doing != nil && *u.Done && *u.Doing {
		return Task{}, fmt.Errorf("%w: a step can't be done and in progress", ErrInvalid)
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, err
	}
	if !mayManage(t, by) {
		return Task{}, fmt.Errorf("%w: tick a step of task %s", ErrNotAllowed, id)
	}
	i := slices.IndexFunc(t.Steps, func(s Step) bool { return s.ID == sid })
	if i < 0 {
		return Task{}, fmt.Errorf("%w: task %s has no step %q", ErrNotFound, id, sid)
	}
	next := applyStep(t.Steps, i, u)
	if slices.Equal(next, t.Steps) {
		return copyTask(t), nil
	}
	if err := c.write(ctx, by, KindTaskStep, taskStepRow{TaskID: id, StepID: sid, Done: u.Done, Doing: u.Doing, By: by, At: c.stamp()}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
}

// applyStep returns ss with u applied to step i (a copy).
func applyStep(ss []Step, i int, u StepUpdate) []Step {
	out := append([]Step(nil), ss...)
	s := &out[i]
	if u.Done != nil {
		s.Done = *u.Done
		if s.Done {
			s.Doing = false
		}
	}
	if u.Doing != nil {
		if *u.Doing {
			for j := range out {
				out[j].Doing = false
			}
			s.Doing, s.Done = true, false
		} else {
			s.Doing = false
		}
	}
	return out
}

// ---- watch ----

func (c *coord) watch(ctx context.Context, by, id string, on bool) (Task, error) {
	by, _, err := callers(by)
	if err != nil {
		return Task{}, err
	}
	if IsPerson(by) || by == sweeper {
		return Task{}, fmt.Errorf("%w: people see every card; only agents watch", ErrInvalid)
	}
	if err := checkKey(by); err != nil {
		return Task{}, err
	}
	c.mu.Lock()
	defer c.unlock()
	var t *Task
	if on {
		t, err = c.openTask(id)
	} else {
		t, err = c.anyTask(id)
	}
	if err != nil {
		return Task{}, err
	}
	if contains(t.Watchers, by) == on {
		return copyTask(t), nil
	}
	if err := c.write(ctx, by, KindTaskWatched, taskWatchedRow{TaskID: id, Who: by, Watching: on, By: by, At: c.stamp()}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
}

// ---- activity ----

func (c *coord) activityOf(id string) []Activity {
	c.mu.Lock()
	defer c.unlock()
	if c.tasks[id] == nil {
		return nil
	}
	acts := c.activity[id]
	out := make([]Activity, 0, len(acts))
	for i := len(acts) - 1; i >= 0; i-- {
		out = append(out, acts[i])
	}
	return out
}

// ---- watch digests ----

// watchNotices drains the watch queue into one NoticeWatched per card and
// set of recipients that would read the same lines. The caller holds c.mu.
func (c *coord) watchNotices() []Notice {
	ids := make([]string, 0, len(c.watchq))
	for id := range c.watchq {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	var out []Notice
	for _, id := range ids {
		q := c.watchq[id]
		t := c.tasks[id]
		if t == nil {
			continue
		}
		var who []string
		add := func(k string) {
			if k != "" && !IsPerson(k) && k != sweeper && !contains(who, k) {
				who = append(who, k)
			}
		}
		for _, w := range t.Watchers {
			add(w)
		}
		for _, l := range q.lines {
			for _, a := range l.also {
				add(a)
			}
		}
		byText := map[string]int{}
		for _, r := range who {
			var lines []string
			for _, l := range q.lines {
				if l.by == r || !contains(t.Watchers, r) && !contains(l.also, r) {
					continue
				}
				actor := l.by
				if actor == sweeper {
					actor = "Irori"
				}
				lines = append(lines, actor+" "+l.what)
			}
			if len(lines) == 0 {
				continue
			}
			text := watchText(t, lines, q.dropped)
			if i, ok := byText[text]; ok {
				out[i].To = append(out[i].To, r)
				continue
			}
			byText[text] = len(out)
			out = append(out, Notice{
				Kind: NoticeWatched, TaskID: id, Ref: "task:" + id, Title: t.Title,
				To: []string{r}, By: sweeper, Text: text,
			})
		}
	}
	clear(c.watchq)
	return out
}

const (
	watchHead     = "Update on "
	watchHeadEnd  = ", a card you watch or raised a blocker on:\n"
	watchFoot     = "\nNow: "
	watchEarlierF = "- (%d earlier changes)\n"
)

// MergeWatch folds an earlier NoticeWatched text the recipient hasn't read
// yet into a newer one for the same card: the newer header and "Now:" line,
// the earlier changes first, at most maxWatchLines (the rest counted). ok is
// false when either isn't a watch digest.
func MergeWatch(earlier, newer string) (string, bool) {
	oLines, oMore, _, _, ok1 := watchParts(earlier)
	nLines, nMore, head, foot, ok2 := watchParts(newer)
	if !ok1 || !ok2 {
		return "", false
	}
	lines := append(oLines, nLines...)
	more := oMore + nMore
	if len(lines) > maxWatchLines {
		more += len(lines) - maxWatchLines
		lines = lines[len(lines)-maxWatchLines:]
	}
	var b strings.Builder
	b.WriteString(head)
	if more > 0 {
		fmt.Fprintf(&b, watchEarlierF, more)
	}
	for _, l := range lines {
		b.WriteString(l)
	}
	b.WriteString(foot)
	return b.String(), true
}

// watchParts splits a watchText result into its change lines (each ending
// in a newline), the earlier-changes count, the header and the "Now:" tail.
func watchParts(s string) (lines []string, more int, head, foot string, ok bool) {
	i, j := strings.Index(s, watchHeadEnd), strings.LastIndex(s, watchFoot)
	if !strings.HasPrefix(s, watchHead) || i < 0 || j < i+len(watchHeadEnd)-1 {
		return nil, 0, "", "", false
	}
	head, foot = s[:i+len(watchHeadEnd)], s[j+1:]
	for _, l := range strings.SplitAfter(s[i+len(watchHeadEnd):j+1], "\n") {
		var n int
		switch {
		case l == "":
		case strings.HasPrefix(l, "- (") && len(lines) == 0 && more == 0:
			if _, err := fmt.Sscanf(l, watchEarlierF, &n); err == nil {
				more = n
				continue
			}
			lines = append(lines, l)
		case !strings.HasPrefix(l, "- ") && len(lines) > 0:
			lines[len(lines)-1] += l // a change whose text ran onto another line
		default:
			lines = append(lines, l)
		}
	}
	return lines, more, head, foot, true
}

func watchText(t *Task, lines []string, dropped int) string {
	var b strings.Builder
	fmt.Fprintf(&b, watchHead+"%q (task:%s)"+watchHeadEnd, t.Title, t.ID)
	shown := lines
	if len(shown) > maxWatchLines {
		shown = shown[len(shown)-maxWatchLines:]
	}
	if more := len(lines) - len(shown) + dropped; more > 0 {
		fmt.Fprintf(&b, watchEarlierF, more)
	}
	for _, l := range shown {
		b.WriteString("- " + cut(l, maxNoticeField) + "\n")
	}
	fmt.Fprintf(&b, "Now: %s", t.State)
	if d, n := t.Progress(); n > 0 {
		fmt.Fprintf(&b, ", steps %d/%d", d, n)
	}
	if n := countOpen(t); n > 0 {
		fmt.Fprintf(&b, ", %d open blockers", n)
	}
	b.WriteString(".")
	b.WriteString(deeper(t.ID))
	return b.String()
}

// ---- PR links ----

var prPathRe = regexp.MustCompile(`^/([\w.-]+)/([\w.-]+)/pull/([0-9]+)(?:/|$)`)

func prLink(l Link) (string, int, bool) {
	if l.Kind != LinkURL {
		return "", 0, false
	}
	u, err := url.Parse(l.Ref)
	if err != nil || strings.TrimPrefix(strings.ToLower(u.Hostname()), "www.") != "github.com" {
		return "", 0, false
	}
	m := prPathRe.FindStringSubmatch(u.Path)
	if m == nil {
		return "", 0, false
	}
	n, err := strconv.Atoi(m[3])
	if err != nil || n <= 0 {
		return "", 0, false
	}
	return m[1] + "/" + m[2], n, true
}

// blockQuestion turns a Block reason (up to MaxReason) into an item's
// question and, when the reason is longer than a question, its context.
func blockQuestion(reason string) (string, string) {
	if utf8.RuneCountInString(reason) <= MaxQuestion {
		return reason, ""
	}
	return cut(reason, MaxQuestion), reason
}

// applyCard folds the cards' own rows. Rows are applied as written.
func (c *coord) applyCard(ev store.Event, at func(time.Time) time.Time) {
	switch ev.Kind {
	case KindBlockerOption:
		var p blockerOptionRow
		if json.Unmarshal(ev.Payload, &p) != nil || p.Answer == "" {
			return
		}
		t := c.tasks[p.TaskID]
		if t == nil {
			return
		}
		i := blockerIndex(t, p.BlockerID)
		if i < 0 || !t.Blockers[i].Open() || len(t.Blockers[i].Options) >= MaxOptions || optionIndex(t.Blockers[i].Options, p.Answer) >= 0 {
			return
		}
		b := &t.Blockers[i]
		when := at(p.At)
		b.Options = append(b.Options, Option{Answer: p.Answer, Why: p.Why, By: p.By, At: when})
		t.UpdatedAt = when
		c.touch(t, when, p.By, "recommended “"+p.Answer+"” for “"+b.Question+"”", true, b.RaisedBy)
	case KindBlockerResolved:
		var p blockerResolvedRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		t := c.tasks[p.TaskID]
		if t == nil {
			return
		}
		i := blockerIndex(t, p.BlockerID)
		if i < 0 || !t.Blockers[i].Open() {
			return
		}
		when := at(p.At)
		opt := p.Option
		if opt < -1 || opt >= len(t.Blockers[i].Options) {
			opt = -1
		}
		q := t.Blockers[i].Question
		t.Blockers[i].Resolved = &Resolution{Answer: p.Answer, Option: opt, By: p.By, At: when, Closed: p.Closed}
		if t.State == Blocked && countOpen(t) == 0 {
			s := p.Restored
			if !ValidState(s) || s == Blocked || IsTerminal(s) {
				s = restoreState(t)
			}
			t.State = s
		}
		trimResolved(t)
		syncReason(t)
		t.UpdatedAt, t.UpdatedBy = when, p.By
		if p.Closed {
			c.touch(t, when, p.By, "closed “"+q+"”: "+p.Answer, true)
		} else {
			c.touch(t, when, p.By, "answered “"+q+"”: "+p.Answer, true)
		}
	case KindTaskSteps:
		var p taskStepsRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		t := c.tasks[p.TaskID]
		if t == nil {
			return
		}
		var ss []Step
		for _, s := range p.Steps {
			if s.ID == "" || len(ss) >= MaxSteps || slices.ContainsFunc(ss, func(o Step) bool { return o.ID == s.ID }) {
				continue
			}
			c.stepSeq[t.ID] = max(c.stepSeq[t.ID], seqOf(s.ID, "s"))
			ss = append(ss, s)
		}
		t.Steps = nil
		if len(ss) > 0 {
			t.Steps = normSteps(ss)
		}
		when := at(p.At)
		t.UpdatedAt, t.UpdatedBy = when, p.By
		d, n := t.Progress()
		c.touch(t, when, p.By, fmt.Sprintf("set the steps (%d/%d done)", d, n), true)
	case KindTaskStep:
		var p taskStepRow
		if json.Unmarshal(ev.Payload, &p) != nil {
			return
		}
		t := c.tasks[p.TaskID]
		if t == nil {
			return
		}
		i := slices.IndexFunc(t.Steps, func(s Step) bool { return s.ID == p.StepID })
		if i < 0 {
			return
		}
		before := t.Steps[i]
		t.Steps = applyStep(t.Steps, i, StepUpdate{Done: p.Done, Doing: p.Doing})
		after := t.Steps[i]
		when := at(p.At)
		t.UpdatedAt, t.UpdatedBy = when, p.By
		what := ""
		switch {
		case after.Done && !before.Done:
			what = "ticked “" + after.Text + "”"
		case !after.Done && before.Done:
			what = "unticked “" + after.Text + "”"
		case after.Doing && !before.Doing:
			what = "started “" + after.Text + "”"
		case !after.Doing && before.Doing:
			what = "paused “" + after.Text + "”"
		}
		if what != "" {
			d, n := t.Progress()
			what += fmt.Sprintf(" (%d/%d)", d, n)
		}
		c.touch(t, when, p.By, what, true)
	case KindTaskWatched:
		var p taskWatchedRow
		if json.Unmarshal(ev.Payload, &p) != nil || p.Who == "" {
			return
		}
		t := c.tasks[p.TaskID]
		if t == nil || contains(t.Watchers, p.Who) == p.Watching {
			return
		}
		what := "stopped watching"
		if p.Watching {
			t.Watchers = append(t.Watchers, p.Who)
			what = "started watching"
		} else {
			t.Watchers = removeStr(t.Watchers, p.Who)
		}
		c.touch(t, at(p.At), p.Who, what, false)
	default:
		return
	}
	c.note(Change{Kind: TaskChanged, TaskID: taskOf(ev)})
}

// taskOf reads a row's taskId.
func taskOf(ev store.Event) string {
	var p struct {
		TaskID string `json:"taskId"`
	}
	_ = json.Unmarshal(ev.Payload, &p)
	return p.TaskID
}

// ---- copies ----

func copyBlocker(b Blocker) Blocker {
	if b.Options != nil {
		b.Options = append([]Option(nil), b.Options...)
	}
	if b.Resolved != nil {
		r := *b.Resolved
		b.Resolved = &r
	}
	b.On = copyOn(b.On)
	return b
}

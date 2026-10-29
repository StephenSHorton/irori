package coord

import (
	"context"
	"fmt"
	"slices"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/store"
)

// Subtasks (docs/design/board.md "Subtasks"): a card can have a parent, one
// level deep. Links come from task.created's parent and from task.parent
// rows; relink applies them in (hlc, device, seq) order with the one-level
// rule, and rollup derives each parent's progress and state from its
// subtasks. Nothing derived is written.

// KindTaskParent sets or clears a card's parent: {taskId, parent, by}
// (parent "" clears it).
const KindTaskParent = "task.parent"

type taskParentRow struct {
	TaskID string    `json:"taskId"`
	Parent string    `json:"parent"`
	By     string    `json:"by"`
	At     time.Time `json:"at"`
}

// parentLink is one link row as folded.
type parentLink struct {
	child, parent string // parent "" clears
	at            store.Cursor
}

// Rollup is a parent's progress and state, derived from its subtasks.
type Rollup struct {
	// State is the parent's state as the Board shows it (EffectiveState).
	State TaskState `json:"state"`
	// Done counts completed subtasks; Total those not canceled.
	Done  int `json:"done"`
	Total int `json:"total"`
	// Blocked is the first blocked subtask's id, when the parent shows
	// blocked because of it; BlockedQuestion is that subtask's first open
	// question.
	Blocked         string `json:"blocked,omitempty"`
	BlockedQuestion string `json:"blockedQuestion,omitempty"`
}

// EffectiveState is t's state as readers show it: a parent's comes from
// its subtasks (Rollup), any other card's is its own.
func EffectiveState(t Task) TaskState {
	if t.Rollup != nil {
		return t.Rollup.State
	}
	return t.State
}

// foldParent records a link row. c.mu is held.
func (c *coord) foldParent(child, parent string, ev store.Event) {
	if child == "" {
		return
	}
	c.links = append(c.links, parentLink{child: child, parent: parent, at: ev.Cursor()})
}

// askAt reports whether id is an ask that was still open at position at:
// an open ask, or one closed by a row after at. Such a card is neither a
// parent nor a subtask there (the take lock's position rule, ask.go).
func (c *coord) askAt(id string, at store.Cursor) bool {
	t := c.tasks[id]
	if t == nil || t.Ask == nil {
		return false
	}
	if t.Ask.Open() {
		return true
	}
	return cursorLess(at, c.asks.closedAt[id])
}

// relink derives every card's Parent and Subtasks from the link rows, then
// each parent's Rollup. Rows apply in log order; one that would make two
// levels, links a card to itself or names an ask still open there is
// ignored. Outside replay a card whose parent or roll-up changed is noted.
// c.mu is held.
func (c *coord) relink() {
	sort.SliceStable(c.links, func(i, j int) bool { return cursorLess(c.links[i].at, c.links[j].at) })
	parentOf := map[string]string{}
	kids := map[string]int{}
	for _, l := range c.links {
		p := l.parent
		if p != "" {
			switch {
			case p == l.child, c.tasks[l.child] == nil, c.tasks[p] == nil,
				parentOf[p] != "", kids[l.child] > 0, c.askAt(l.child, l.at), c.askAt(p, l.at):
				continue // inert: the card keeps the parent it had
			}
		}
		if old, ok := parentOf[l.child]; ok {
			kids[old]--
			delete(parentOf, l.child)
		}
		if p != "" {
			parentOf[l.child] = p
			kids[p]++
		}
	}
	subs := map[string][]string{}
	for child, p := range parentOf {
		subs[p] = append(subs[p], child)
	}
	for id, t := range c.tasks {
		ks := subs[id]
		sort.Slice(ks, func(i, j int) bool { return c.subtaskLess(ks[i], ks[j]) })
		was, wasKids, wasRoll := t.Parent, t.Subtasks, t.Rollup
		t.Parent, t.Subtasks, t.Rollup = parentOf[id], ks, nil
		if len(ks) > 0 {
			t.Rollup = c.rollup(t, ks)
		}
		if !c.replaying && (was != t.Parent || !slices.Equal(wasKids, t.Subtasks) || !sameRollup(wasRoll, t.Rollup)) {
			c.note(Change{Kind: TaskChanged, TaskID: id})
		}
	}
}

// subtaskLess orders subtasks by short number (then letter), then id.
func (c *coord) subtaskLess(a, b string) bool {
	sa, sb := c.tasks[a].Short, c.tasks[b].Short
	na, nb := shortInt(sa), shortInt(sb)
	if na != nb {
		return na < nb
	}
	if sa != sb {
		return sa < sb
	}
	return a < b
}

// shortInt is a short ref's number ("12b" is 12); a card without one
// sorts last.
func shortInt(short string) int {
	n, err := strconv.Atoi(shortNum(short))
	if err != nil {
		return 1 << 30
	}
	return n
}

func sameRollup(a, b *Rollup) bool {
	if a == nil || b == nil {
		return a == b
	}
	return *a == *b
}

// rollup derives parent t's progress and state from subtasks ks. The
// parent's own state wins when it has ended (canceled with them) or when
// its own question is open (blocked).
func (c *coord) rollup(t *Task, ks []string) *Rollup {
	r := &Rollup{}
	ended, completed, failed := 0, 0, 0
	var blocked, input, working, started bool
	for _, id := range ks {
		k := c.tasks[id]
		switch k.State {
		case Completed:
			r.Done++
			completed++
			ended++
		case Canceled:
			ended++
			continue // canceled subtasks don't count toward the total
		case Failed:
			failed++
			ended++
		case Blocked:
			blocked = true
			if r.Blocked == "" {
				r.Blocked = id
				r.BlockedQuestion = firstOpenQuestion(k)
			}
		case InputRequired:
			input = true
		case Working:
			working = true
		}
		if k.State != Submitted {
			started = true
		}
		r.Total++
	}
	switch {
	case IsTerminal(t.State):
		r.State = t.State
	case ended == len(ks) && completed > 0:
		r.State = Completed
	case ended == len(ks) && failed > 0:
		r.State = Failed
	case ended == len(ks):
		r.State = Canceled
	case t.State == Blocked || blocked:
		r.State = Blocked
		if t.State == Blocked {
			r.Blocked, r.BlockedQuestion = "", "" // its own question comes first
		}
	case input || t.State == InputRequired:
		r.State = InputRequired
	case working || started:
		r.State = Working
	default:
		r.State = Submitted
	}
	return r
}

func firstOpenQuestion(t *Task) string {
	for _, b := range t.Blockers {
		if b.Open() {
			return b.Question
		}
	}
	return t.Reason
}

// ---- write-time checks ----

// ErrSubtask: a parent or subtask rule refused the call (one level, open
// asks, a parent's derived state or steps).
var ErrSubtask = fmt.Errorf("%w: subtasks", ErrInvalid)

// checkParent is whether parent may take a new or adopted subtask child
// ("" for a card not made yet). c.mu is held.
func (c *coord) checkParent(parentID, child string) (*Task, error) {
	p := c.tasks[parentID]
	if p == nil {
		return nil, fmt.Errorf("%w: parent %q", ErrNotFound, parentID)
	}
	switch {
	case parentID == child:
		return nil, fmt.Errorf("%w: a card can't be its own subtask", ErrSubtask)
	case openAsk(p) != nil:
		return nil, fmt.Errorf("%w: %s is an open ask; ask_claim it and ask_close it as a card first", ErrSubtask, p.ShortRef())
	case p.Parent != "":
		return nil, fmt.Errorf("%w: %s is itself a subtask of %s, and subtasks are one level; add a sibling with board_create parent=%s",
			ErrSubtask, p.ShortRef(), c.refOf(p.Parent), c.refOf(p.Parent))
	case IsTerminal(p.State):
		return nil, fmt.Errorf("%w: %s is %s", ErrBadState, p.ShortRef(), p.State)
	}
	if child == "" {
		return p, nil
	}
	k := c.tasks[child]
	if k == nil {
		return nil, fmt.Errorf("%w: task %q", ErrNotFound, child)
	}
	switch {
	case openAsk(k) != nil:
		return nil, fmt.Errorf("%w: %s is an open ask; ask_close it as a card (with parent) instead", ErrSubtask, k.ShortRef())
	case IsTerminal(k.State):
		return nil, fmt.Errorf("%w: %s is %s; only cards still in play can be adopted", ErrBadState, k.ShortRef(), k.State)
	case len(k.Subtasks) > 0:
		return nil, fmt.Errorf("%w: %s has subtasks of its own, and subtasks are one level", ErrSubtask, k.ShortRef())
	case k.Parent == parentID:
		return nil, fmt.Errorf("%w: %s is already a subtask of %s", ErrInvalid, k.ShortRef(), p.ShortRef())
	}
	return p, nil
}

// refOf is id's short ref, or its task: ref. c.mu is held.
func (c *coord) refOf(id string) string {
	if t := c.tasks[id]; t != nil {
		return t.ShortRef()
	}
	return "task:" + id
}

// parentStateRefusal is setTaskState's rule for a parent: canceled cancels
// it and its open subtasks (cascade true); completed only once every
// subtask has ended; nothing else, since its state follows its subtasks.
// c.mu is held.
func (c *coord) parentStateRefusal(t *Task, s TaskState) (cascade bool, err error) {
	if len(t.Subtasks) == 0 {
		return false, nil
	}
	switch s {
	case Canceled:
		return true, nil
	case Completed:
		var open []string
		for _, id := range t.Subtasks {
			if k := c.tasks[id]; !IsTerminal(k.State) {
				open = append(open, k.ShortRef()+" ("+string(k.State)+")")
			}
		}
		if len(open) > 0 {
			return false, fmt.Errorf("%w: %s still has open subtasks: %s. Finish or cancel them; the parent completes with them",
				ErrSubtask, t.ShortRef(), strings.Join(open, ", "))
		}
		return false, nil
	}
	return false, fmt.Errorf("%w: %s is a parent: its state follows its subtasks (only canceled or completed can be set)", ErrSubtask, t.ShortRef())
}

// ---- split ----

// NewSubtask is one subtask Split makes. Steps are step ids of the card
// being split (they move) or new step texts. Assigning is the caller's
// (Assign, after Split), so its notice goes out as any assignment's.
type NewSubtask struct {
	Title, Objective, DoneCriteria string
	Steps                          []string
}

// Split turns card id into a parent: each NewSubtask becomes a subtask
// holding the steps given to it, and every card in adopt (full ids) becomes
// a subtask too. Every open step of id must go to exactly one subtask; its
// done steps stay on it. Returns the parent and the new subtasks.
func (c *Coord) Split(ctx context.Context, by, id string, subs []NewSubtask, adopt []string) (Task, []Task, error) {
	return c.impl.split(ctx, by, id, subs, adopt)
}

func (c *coord) split(ctx context.Context, by, id string, subs []NewSubtask, adopt []string) (Task, []Task, error) {
	by, _, err := callers(by)
	if err != nil {
		return Task{}, nil, err
	}
	if len(subs) == 0 && len(adopt) == 0 {
		return Task{}, nil, fmt.Errorf("%w: give subtasks or cards to adopt", ErrInvalid)
	}
	if len(subs)+len(adopt) > maxSubtasks {
		return Task{}, nil, fmt.Errorf("%w: at most %d subtasks in one split", ErrInvalid, maxSubtasks)
	}
	c.mu.Lock()
	defer c.unlock()
	t, err := c.openTask(id)
	if err != nil {
		return Task{}, nil, err
	}
	if _, err := c.checkParent(id, ""); err != nil {
		return Task{}, nil, err
	}
	if !c.mayManage(t, by) {
		return Task{}, nil, fmt.Errorf("%w: split task %s", ErrNotAllowed, t.ShortRef())
	}
	// Place the steps: an existing open step id moves; any other text is
	// a new step.
	byID := map[string]Step{}
	for _, s := range t.Steps {
		byID[s.ID] = s
	}
	placed := map[string]bool{}
	made := make([]NewTask, len(subs))
	for i, s := range subs {
		nt := NewTask{Title: s.Title, Objective: s.Objective, DoneCriteria: s.DoneCriteria}
		for _, raw := range s.Steps {
			txt := strings.TrimSpace(raw)
			if old, ok := byID[txt]; ok {
				switch {
				case old.Done:
					return Task{}, nil, fmt.Errorf("%w: step %s is done; done steps stay on the parent", ErrInvalid, txt)
				case placed[txt]:
					return Task{}, nil, fmt.Errorf("%w: step %s is given to two subtasks", ErrInvalid, txt)
				}
				placed[txt] = true
				nt.Steps = append(nt.Steps, Step{Text: old.Text, Doing: old.Doing})
				continue
			}
			nt.Steps = append(nt.Steps, Step{Text: txt})
		}
		made[i] = nt
	}
	var unplaced []string
	for _, s := range t.Steps {
		if !s.Done && !placed[s.ID] {
			unplaced = append(unplaced, s.ID+" "+strconv.Quote(s.Text))
		}
	}
	if len(unplaced) > 0 {
		return Task{}, nil, fmt.Errorf("%w: every open step must go to a subtask; not placed: %s", ErrInvalid, strings.Join(unplaced, ", "))
	}
	cleaned := make([]cleanTask, len(made))
	for i, nt := range made {
		ct, err := cleanNew(by, nt)
		if err != nil {
			return Task{}, nil, fmt.Errorf("subtask %d: %w", i+1, err)
		}
		ct.parent = id
		cleaned[i] = ct
	}
	adoptIDs := make([]string, 0, len(adopt))
	for _, a := range adopt {
		k, err := c.taskRefLocked(a)
		if err != nil {
			return Task{}, nil, err
		}
		if _, err := c.checkParent(id, k); err != nil {
			return Task{}, nil, err
		}
		if kt := c.tasks[k]; !c.mayManage(kt, by) && !c.mayManage(t, by) {
			return Task{}, nil, fmt.Errorf("%w: adopt %s", ErrNotAllowed, kt.ShortRef())
		}
		if slices.Contains(adoptIDs, k) {
			continue
		}
		adoptIDs = append(adoptIDs, k)
	}

	var out []Task
	for _, ct := range cleaned {
		k, err := c.createLocked(ctx, by, ct)
		if err != nil {
			return Task{}, nil, err
		}
		out = append(out, k)
	}
	for _, k := range adoptIDs {
		if err := c.write(ctx, by, KindTaskParent, taskParentRow{TaskID: k, Parent: id, By: by, At: c.stamp()}); err != nil {
			return Task{}, nil, err
		}
	}
	if len(placed) > 0 {
		var keep []Step
		for _, s := range t.Steps {
			if s.Done {
				keep = append(keep, s)
			}
		}
		if err := c.write(ctx, by, KindTaskSteps, taskStepsRow{TaskID: id, Steps: keep, By: by, At: c.stamp()}); err != nil {
			return Task{}, nil, err
		}
	}
	return copyTask(c.tasks[id]), out, nil
}

// maxSubtasks caps one split.
const maxSubtasks = 20

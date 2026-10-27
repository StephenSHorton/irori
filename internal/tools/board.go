package tools

import (
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
)

// The Board tools (docs/design/board.md): a scratch pad for live
// coordination, never a tracker. Every card line carries its ref, and every
// call that someone should hear about routes coord's Notice from the caller.

const maxCardTitle = 160 // a card's title in a list line

// taskID accepts an id with or without its "task:" prefix.
func taskID(v string) string {
	v = strings.TrimSpace(v)
	if k, id, ok := strings.Cut(v, ":"); ok && strings.EqualFold(k, "task") {
		return strings.TrimSpace(id)
	}
	return v
}

func needTaskID() string {
	return errText("give task_id: the card's id as the board tool shows it (task:<id>).")
}

// callerRepo is the caller's repoKey from its card ("" for Grok members).
func (s *set) callerRepo(c Caller) string { return s.roster()[c.Sender.Key].RepoKey }

// cardLine is one card as one list line (two when it is blocked or has a
// last comment), every untrusted field folded.
func (s *set) cardLine(t coord.Task, last *coord.Comment, n names, now time.Time) string {
	var b strings.Builder
	fmt.Fprintf(&b, "- task:%s [%s] %s", oneLine(t.ID, 80), oneLine(string(t.State), 30), quoted(oneLine(t.Title, maxCardTitle), maxCardTitle))
	var holders []string
	for _, h := range t.Holders {
		w := n.who(h)
		if slices.Contains(t.Assigned, h) {
			w += " (assigned, not taken up)"
		}
		holders = append(holders, w)
	}
	if len(holders) > 0 {
		b.WriteString("; held by " + strings.Join(holders, ", "))
	} else {
		b.WriteString("; nobody holds it")
	}
	if t.Shared {
		b.WriteString("; shared")
	}
	if len(t.Links) > 0 {
		var ls []string
		for _, l := range t.Links {
			ls = append(ls, oneLine(l.Label, 60))
		}
		b.WriteString("; links: " + strings.Join(ls, ", "))
	}
	if t.Comments > 0 {
		fmt.Fprintf(&b, "; %d comments", t.Comments)
	}
	b.WriteString("; updated " + ago(now, t.UpdatedAt) + "\n")
	if t.State == coord.Blocked {
		fmt.Fprintf(&b, "  blocked: %s", quoted(t.Reason, maxFieldRunes))
		if bb := t.BlockedBy; bb != nil {
			fmt.Fprintf(&b, " (on %s %s)", oneLine(string(bb.Kind), 20), oneLine(bb.Ref, 160))
		}
		b.WriteString("\n")
	}
	if last != nil {
		fmt.Fprintf(&b, "  last: comment:%s by %s %s: %s\n", oneLine(last.ID, 80), n.who(last.By), ago(now, last.At), quoted(last.Text, 160))
	}
	return b.String()
}

// --- board ---

type boardArgs struct {
	Repo string `json:"repo"`
	Done bool   `json:"done"`
}

func (s *set) board(_ context.Context, c Caller, raw json.RawMessage) string {
	var a boardArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	repo := strings.TrimSpace(a.Repo)
	switch repo {
	case "*", "all":
		repo = ""
	case "":
		repo = s.callerRepo(c)
	}
	doneLimit := 1
	if a.Done {
		doneLimit = coord.DefaultDoneCard
	}
	v := s.b.Coord().Board(coord.BoardFilter{RepoKey: repo, For: c.Sender.Key, DoneLimit: doneLimit})
	where := "every repo"
	if repo != "" {
		where = oneLine(repo, 160)
	}
	open := len(v.NeedsYou) + len(v.Blocked) + len(v.Working) + len(v.Open)
	head := fmt.Sprintf("Board for %s: %d open cards, %d done", where, open, v.DoneTotal)
	if open == 0 && len(v.Leases) == 0 && (!a.Done || v.DoneTotal == 0) {
		return head + ". Nothing live here; board_create puts up a card for work starting now."
	}
	n := s.roster()
	now := s.now()
	var b strings.Builder
	b.WriteString(head + ". " + untrusted + "\n")
	const hintRoom = 200
	budget := MaxResult - hintRoom
	cut := 0
	group := func(name string, cards []coord.Card) {
		if len(cards) == 0 {
			return
		}
		fmt.Fprintf(&b, "\n%s (%d):\n", name, len(cards))
		for _, cd := range cards {
			line := s.cardLine(cd.Task, cd.LastComment, n, now)
			if cd.Resolved {
				line += "  its blocker has gone away; it closes on the next sweep, or board_unblock now\n"
			}
			if len(cd.Overlaps) > 0 {
				line += "  holders' leases overlap: " + oneLine(strings.Join(cd.Overlaps, "; "), maxFieldRunes) + "\n"
			}
			if cut > 0 || b.Len()+len(line) > budget {
				cut++
				continue
			}
			b.WriteString(line)
		}
	}
	group("Needs you", v.NeedsYou)
	group("Blocked", v.Blocked)
	group("Working", v.Working)
	group("Open", v.Open)
	if len(v.Leases) > 0 && cut == 0 {
		fmt.Fprintf(&b, "\nLeases in the way (%d):\n", len(v.Leases))
		for _, l := range v.Leases {
			line := fmt.Sprintf("- %s held by %s, expires %s", oneLine(l.Resource, maxFieldRunes), n.who(l.Holder), in(now, l.Until))
			if len(l.Blocks) > 0 {
				line += "; blocks task:" + oneLine(strings.Join(l.Blocks, ", task:"), maxFieldRunes)
			}
			var refused []string
			for _, r := range l.Refused {
				refused = append(refused, n.who(r.By)+" "+ago(now, r.At))
			}
			if len(refused) > 0 {
				line += "; refused to " + strings.Join(refused, ", ")
			}
			line += "\n"
			if b.Len()+len(line) > budget {
				cut++
				continue
			}
			b.WriteString(line)
		}
	}
	if a.Done {
		group("Done, newest first", v.Done)
	}
	if cut > 0 {
		fmt.Fprintf(&b, "\n(%d more lines not shown; narrow with repo, or search kinds=[\"task\"] with words from the card)\n", cut)
	}
	switch {
	case !a.Done && v.DoneTotal > 0:
		fmt.Fprintf(&b, "\n%d done cards are collapsed: board done = true lists the newest %d. ", v.DoneTotal, coord.DefaultDoneCard)
	case a.Done && v.DoneTotal > len(v.Done):
		fmt.Fprintf(&b, "\n%d older done cards not listed: search kinds=[\"task\"] finds them. ", v.DoneTotal-len(v.Done))
	default:
		b.WriteString("\n")
	}
	b.WriteString("show task:<id> for a card's comments and links.\n")
	return b.String()
}

// --- board_create ---

type boardCreateArgs struct {
	Title     string   `json:"title"`
	Objective string   `json:"objective"`
	DoneWhen  string   `json:"done_when"`
	Repo      string   `json:"repo"`
	Shared    bool     `json:"shared"`
	Links     []string `json:"links"`
	AssignTo  string   `json:"assign_to"`
}

func (s *set) boardCreate(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardCreateArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	if strings.TrimSpace(a.Title) == "" {
		return errText("give the card a title: what is being worked on now.")
	}
	repo := strings.TrimSpace(a.Repo)
	if repo == "" {
		repo = s.callerRepo(c)
	}
	var to string
	if v := strings.TrimSpace(a.AssignTo); v != "" {
		k, err := s.assignee(v)
		if err != nil {
			return mapErr(err)
		}
		to = k
	}
	t, err := s.b.Coord().CreateTask(ctx, c.Sender.Key, coord.NewTask{
		Title: strings.TrimSpace(a.Title), Objective: strings.TrimSpace(a.Objective),
		DoneCriteria: strings.TrimSpace(a.DoneWhen), RepoKey: repo, Shared: a.Shared, Links: a.Links,
	})
	if err != nil {
		return mapErr(err)
	}
	n := s.roster()
	out := "Created task:" + t.ID + "."
	if to != "" {
		at, notice, err := s.b.Coord().Assign(ctx, c.Sender.Key, t.ID, to)
		if err != nil {
			return out + " Assigning it failed: " + strings.TrimPrefix(mapErr(err), "Error: ") + "\n" + s.cardLine(t, nil, n, s.now())
		}
		t = at
		out += " Assigned to " + n.who(to) + "." + s.told(RouteNotice(ctx, s.b.Inbox(), c.Sender, notice), n)
	} else {
		out += " Take it up with board_join, or hand it over with board_assign."
	}
	return out + "\n" + s.cardLine(t, nil, n, s.now())
}

// assignee resolves an agent name or key (live or gone, the caller too),
// or passes a person's "user:*" key through.
func (s *set) assignee(v string) (string, error) {
	if strings.HasPrefix(v, "user:") {
		return v, nil
	}
	return s.resolveAgent(v)
}

// --- board_comment ---

type boardCommentArgs struct {
	TaskID string `json:"task_id"`
	Text   string `json:"text"`
	Wake   bool   `json:"wake"`
}

func (s *set) boardComment(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardCommentArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := taskID(a.TaskID)
	if id == "" {
		return needTaskID()
	}
	if strings.TrimSpace(a.Text) == "" {
		return errText("give the comment text.")
	}
	cm, notice, err := s.b.Coord().AddComment(ctx, c.Sender.Key, id, strings.TrimSpace(a.Text), a.Wake)
	if err != nil {
		return mapErr(err)
	}
	n := s.roster()
	out := "Commented comment:" + cm.ID + " on task:" + id + "."
	if rs := RouteNotice(ctx, s.b.Inbox(), c.Sender, notice); len(rs) > 0 {
		out += s.told(rs, n)
	} else {
		out += " Nobody else holds it, so nobody was told; it shows on the Board."
	}
	return out
}

// --- board_block / board_unblock ---

type boardBlockArgs struct {
	TaskID string `json:"task_id"`
	Reason string `json:"reason"`
	On     string `json:"on"`
}

// parseOn reads board_block's on: "<kind> <ref>" or "<kind>:<ref>".
func (s *set) parseOn(v string) (*coord.BlockedBy, string) {
	v = strings.TrimSpace(v)
	if v == "" {
		return nil, ""
	}
	cut := strings.IndexAny(v, ": ")
	kind, ref := v, ""
	if cut >= 0 {
		kind, ref = v[:cut], strings.TrimSpace(v[cut+1:])
	}
	bb := &coord.BlockedBy{Kind: coord.BlockerKind(strings.ToLower(kind)), Ref: ref}
	switch bb.Kind {
	case coord.BlockedOnLease:
		if ref == "" {
			return nil, errText("on = lease needs the resource, e.g. lease path:<repoKey>:src/api/** (see the leases tool).")
		}
	case coord.BlockedOnTask:
		if bb.Ref = taskID(ref); bb.Ref == "" {
			return nil, errText("on = task needs its id, e.g. task 01J….")
		}
	case coord.BlockedOnPerson:
		if ref != "" && !strings.HasPrefix(ref, "user:") {
			return nil, errText("on = person takes no name (whoever is at the app), or a user:* key.")
		}
	case coord.BlockedOnAgent:
		if ref == "" {
			return nil, errText("on = agent needs the agent's name or key.")
		}
		k, err := s.resolveAgent(ref)
		if err != nil {
			return nil, mapErr(err)
		}
		bb.Ref = k
	case coord.BlockedOnExternal:
	default:
		return nil, errText(fmt.Sprintf("on starts with what it waits on: lease, task, person, agent or external, not %q.", oneLine(kind, 40)))
	}
	return bb, ""
}

func (s *set) boardBlock(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardBlockArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := taskID(a.TaskID)
	if id == "" {
		return needTaskID()
	}
	if strings.TrimSpace(a.Reason) == "" {
		return errText("give reason: what is in the way.")
	}
	bb, msg := s.parseOn(a.On)
	if msg != "" {
		return msg
	}
	t, err := s.b.Coord().Block(ctx, c.Sender.Key, id, strings.TrimSpace(a.Reason), bb)
	if err != nil {
		return mapErr(err)
	}
	out := "Blocked task:" + t.ID + "."
	if bb != nil && (bb.Kind == coord.BlockedOnLease || bb.Kind == coord.BlockedOnTask) {
		out += " It unblocks by itself when that " + string(bb.Kind) + " goes away."
	} else {
		out += " Anyone may board_unblock it."
	}
	return out + "\n" + s.cardLine(t, nil, s.roster(), s.now())
}

type boardUnblockArgs struct {
	TaskID string `json:"task_id"`
	Note   string `json:"note"`
}

func (s *set) boardUnblock(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardUnblockArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := taskID(a.TaskID)
	if id == "" {
		return needTaskID()
	}
	t, notice, err := s.b.Coord().Unblock(ctx, c.Sender.Key, id, strings.TrimSpace(a.Note))
	if err != nil {
		return mapErr(err)
	}
	n := s.roster()
	return "Unblocked task:" + t.ID + "; it is " + string(t.State) + " again." +
		s.told(RouteNotice(ctx, s.b.Inbox(), c.Sender, notice), n) + "\n" + s.cardLine(t, nil, n, s.now())
}

// --- board_join / board_leave / board_assign ---

type boardTaskArgs struct {
	TaskID string `json:"task_id"`
}

func (s *set) boardJoin(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardTaskArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := taskID(a.TaskID)
	if id == "" {
		return needTaskID()
	}
	co := s.b.Coord()
	cur, ok := co.GetTask(id)
	if !ok {
		return mapErr(fmt.Errorf("%w: task %s", coord.ErrNotFound, oneLine(id, 80)))
	}
	var t coord.Task
	var err error
	if cur.Shared {
		t, err = co.Join(ctx, c.Sender.Key, id, c.Sender.Key)
	} else {
		t, err = co.ClaimTask(ctx, c.Sender.Key, id)
	}
	if err != nil {
		return mapErr(err)
	}
	out := "You hold task:" + t.ID + " now"
	if t.Shared && len(t.Holders) > 1 {
		out += ", with the others on it; leases keep you off each other's files"
	}
	return out + ". Post its state with board_status as you go.\n" + s.cardLine(t, nil, s.roster(), s.now())
}

func (s *set) boardLeave(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardTaskArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := taskID(a.TaskID)
	if id == "" {
		return needTaskID()
	}
	t, err := s.b.Coord().Leave(ctx, c.Sender.Key, id, c.Sender.Key)
	if err != nil {
		return mapErr(err)
	}
	return "You left task:" + t.ID + "; it stays on the Board.\n" + s.cardLine(t, nil, s.roster(), s.now())
}

type boardAssignArgs struct {
	TaskID string `json:"task_id"`
	To     string `json:"to"`
}

func (s *set) boardAssign(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardAssignArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := taskID(a.TaskID)
	if id == "" {
		return needTaskID()
	}
	want := strings.TrimSpace(a.To)
	if want == "" {
		return errText("give to: the agent's name or key (see the agents tool).")
	}
	to, err := s.assignee(want)
	if err != nil {
		return mapErr(err)
	}
	t, notice, err := s.b.Coord().Assign(ctx, c.Sender.Key, id, to)
	if err != nil {
		return mapErr(err)
	}
	n := s.roster()
	return "Assigned task:" + t.ID + " to " + n.who(to) + "." + s.told(RouteNotice(ctx, s.b.Inbox(), c.Sender, notice), n) +
		"\n" + s.cardLine(t, nil, n, s.now())
}

// --- board_status / board_link ---

type boardStatusArgs struct {
	TaskID string `json:"task_id"`
	State  string `json:"state"`
	Note   string `json:"note"`
}

var boardStates = []coord.TaskState{coord.Working, coord.InputRequired, coord.Completed, coord.Failed, coord.Canceled}

func (s *set) boardStatus(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardStatusArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := taskID(a.TaskID)
	if id == "" {
		return needTaskID()
	}
	st := coord.TaskState(strings.ToLower(strings.TrimSpace(a.State)))
	switch {
	case st == coord.Blocked:
		return errText("use board_block for blocked: it takes the reason.")
	case !slices.Contains(boardStates, st):
		return errText("give state: working, input_required, completed, failed or canceled.")
	}
	t, err := s.b.Coord().SetTaskState(ctx, c.Sender.Key, id, st, strings.TrimSpace(a.Note))
	if err != nil {
		return mapErr(err)
	}
	out := "task:" + t.ID + " is " + string(t.State) + " now."
	if coord.IsTerminal(t.State) {
		out += " It folds into Done on the Board."
	}
	return out + "\n" + s.cardLine(t, nil, s.roster(), s.now())
}

type boardLinkArgs struct {
	TaskID string `json:"task_id"`
	Link   string `json:"url_or_path"`
}

func (s *set) boardLink(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardLinkArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := taskID(a.TaskID)
	if id == "" {
		return needTaskID()
	}
	if strings.TrimSpace(a.Link) == "" {
		return errText("give url_or_path: an http(s) URL or a repo path.")
	}
	t, err := s.b.Coord().AddLink(ctx, c.Sender.Key, id, a.Link)
	if err != nil {
		return mapErr(err)
	}
	return "Linked task:" + t.ID + ".\n" + s.cardLine(t, nil, s.roster(), s.now())
}

// --- leases ---

type leaseArgs struct {
	Resource   string `json:"resource"`
	TTLMinutes *int   `json:"ttl_minutes"`
	Steal      bool   `json:"steal"`
}

func (s *set) lease(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a leaseArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	res := strings.TrimSpace(a.Resource)
	if res == "" {
		return errText("give resource: path:<repoKey>:<glob> or a name like gpu/qwen.")
	}
	var ttl time.Duration
	if a.TTLMinutes != nil {
		m := *a.TTLMinutes
		if m < 1 || m > int(coord.MaxTTL/time.Minute) {
			return errText(fmt.Sprintf("ttl_minutes must be 1 to %d, not %d.", int(coord.MaxTTL/time.Minute), m))
		}
		ttl = time.Duration(m) * time.Minute
	}
	l, err := s.b.Coord().Acquire(ctx, c.Sender.Key, res, ttl, a.Steal)
	if err != nil {
		if strings.HasPrefix(res, "path:") {
			if rk := s.callerRepo(c); rk != "" {
				return mapErr(err) + " Your repoKey is " + oneLine(rk, 160) + "."
			}
		}
		return mapErr(err)
	}
	n := s.roster()
	out := "Leased " + oneLine(l.Resource, maxFieldRunes) + " until " + l.Until.Format(time.RFC3339) + " (" + in(s.now(), l.Until) + "). Lease again to renew; release when done."
	if l.StolenFrom != "" {
		out += " Taken from " + n.who(l.StolenFrom) + "; consider telling them."
	}
	return out
}

func (s *set) leases(_ context.Context, _ Caller, raw json.RawMessage) string {
	var a struct{}
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	ls := s.b.Coord().Leases()
	if len(ls) == 0 {
		return "No current leases."
	}
	n := s.roster()
	now := s.now()
	var b strings.Builder
	fmt.Fprintf(&b, "%d current leases. %s\n", len(ls), untrusted)
	for _, l := range ls {
		fmt.Fprintf(&b, "- %s held by %s, expires %s", oneLine(l.Resource, maxFieldRunes), n.who(l.Holder), in(now, l.Until))
		if l.StolenFrom != "" {
			fmt.Fprintf(&b, ", taken from %s", n.who(l.StolenFrom))
		}
		b.WriteString("\n")
	}
	return b.String()
}

func (s *set) release(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a struct {
		Resource string `json:"resource"`
	}
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	res := strings.TrimSpace(a.Resource)
	if res == "" {
		return errText("give resource, exactly as the leases tool shows it.")
	}
	if err := s.b.Coord().Release(ctx, c.Sender.Key, res); err != nil {
		return mapErr(err)
	}
	return "Released " + oneLine(res, maxFieldRunes) + "."
}

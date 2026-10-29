package tools

import (
	"cmp"
	"context"
	"encoding/json"
	"errors"
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

// taskID resolves what the caller gave for a card: its short ref ("#12",
// "12", "task:#12") or its id, with or without "task:". msg is the tool's
// answer when it can't be used: none given, or a number that names more
// than one card (refused with each named, so a ref seen before a clash
// settled never acts on the other card). A short ref no card has comes
// back as given, so the call it feeds reports no such task.
func (s *set) taskID(v string) (id, msg string) {
	if strings.TrimSpace(v) == "" {
		return "", needTaskID()
	}
	id, err := s.b.Coord().TaskRef(v)
	switch {
	case errors.Is(err, coord.ErrAmbiguous):
		return "", mapErr(err)
	case err != nil:
		return strings.TrimSpace(v), ""
	}
	return id, ""
}

func needTaskID() string {
	return errText("give task_id: the card's short ref as the board tool shows it (#12), or its task:<id>.")
}

// cardRefs names a card in a list line: "#12 task:<id>", so an agent has
// the short ref to say and the id that never moves.
func cardRefs(t coord.Task) string {
	if t.Short == "" {
		return "task:" + oneLine(t.ID, 80)
	}
	return "#" + oneLine(t.Short, 20) + " task:" + oneLine(t.ID, 80)
}

// shortOf is a card's short ref by id ("#12"), or task:<id> when there is
// no such card.
func (s *set) shortOf(id string) string {
	if t, ok := s.b.Coord().GetTask(id); ok {
		return t.ShortRef()
	}
	return "task:" + oneLine(id, 80)
}

// callerRepo is the caller's repoKey from its card ("" for Grok members).
func (s *set) callerRepo(c Caller) string { return s.roster().cards[c.Sender.Key].RepoKey }

// boardView is the Board for repo ("" every repo) as viewer sees it, with
// NeedsYou taken across every repo: a card assigned to the viewer, or
// blocked on it, needs it wherever the card lives (a Grok member's cards
// have no repo at all).
func (s *set) boardView(repo, viewer string, doneLimit int) coord.BoardView {
	co := s.b.Coord()
	v := co.Board(coord.BoardFilter{RepoKey: repo, For: viewer, DoneLimit: doneLimit})
	if repo != "" {
		v.NeedsYou = co.Board(coord.BoardFilter{For: viewer, DoneLimit: 1}).NeedsYou
	}
	return v
}

// cardLine is one card as one list line (two when it is blocked or has a
// last comment), every untrusted field folded, with every open blocker item.
func (s *set) cardLine(t coord.Task, last *coord.Comment, n names, now time.Time) string {
	return s.cardText(t, last, n, now, false)
}

// Board list lines show this much of a blocked card; the rest is counted.
const (
	listItems   = 1 // open blocker items per card
	listOptions = 3 // recommended answers per item
)

// cardText is cardLine; brief (the board list) shows only the first open
// item and its first options, so one heavily blocked card can't fill the
// list's budget and push every other card off it.
func (s *set) cardText(t coord.Task, last *coord.Comment, n names, now time.Time, brief bool) string {
	var b strings.Builder
	state := string(t.State)
	if t.Ask.Open() {
		state = "ask"
	}
	fmt.Fprintf(&b, "- %s [%s] %s", cardRefs(t), oneLine(state, 30), quoted(oneLine(t.Title, maxCardTitle), maxCardTitle))
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
	if len(t.Watchers) > 0 {
		var ws []string
		for _, w := range t.Watchers {
			ws = append(ws, n.who(w))
		}
		b.WriteString("; watched by " + strings.Join(ws, ", "))
	}
	b.WriteString("; updated " + ago(now, t.UpdatedAt) + "\n")
	if done, total := t.Progress(); total > 0 {
		fmt.Fprintf(&b, "  steps %d/%d", done, total)
		for _, st := range t.Steps {
			if st.Doing {
				fmt.Fprintf(&b, ", now %s %s", oneLine(st.ID, 20), quoted(oneLine(st.Text, 120), 120))
			}
		}
		b.WriteString("\n")
	}
	if t.State == coord.Blocked {
		open := t.OpenBlockers()
		if len(open) == 0 {
			// A blocked task always has an item; this is only for safety.
			open = []coord.Blocker{{Question: t.Reason}}
		}
		on := ""
		if bb := t.BlockedBy; bb != nil {
			on = fmt.Sprintf(" (on %s %s)", oneLine(string(bb.Kind), 20), oneLine(bb.Ref, 160))
		}
		more := 0
		if brief && len(open) > listItems {
			more = len(open) - listItems
		}
		for i, bl := range open[:len(open)-more] {
			fmt.Fprintf(&b, "  blocked")
			if bl.ID != "" {
				fmt.Fprintf(&b, " [%s]", oneLine(bl.ID, 20))
			}
			fmt.Fprintf(&b, ": %s", quoted(bl.Question, maxFieldRunes))
			if i == 0 {
				b.WriteString(on)
			}
			if bl.RaisedBy != "" {
				b.WriteString(" raised by " + n.short(bl.RaisedBy))
			}
			switch bl.Status {
			case coord.BlockerAnswered:
				b.WriteString("; answered")
				if to := cmp.Or(bl.SentTo, bl.RaisedBy); to != "" {
					fmt.Fprintf(&b, " (waiting on %s: resolve or reply)", n.short(to))
				}
			case coord.BlockerReplied:
				b.WriteString("; replied (with the person)")
				if len(bl.Missing) > 0 {
					var ms []string
					for _, m := range bl.Missing {
						ms = append(ms, quoted(m, coord.MaxMissingText))
					}
					b.WriteString(", missing: " + strings.Join(ms, ", "))
				}
			}
			b.WriteString("\n")
			opts := bl.Options
			if brief && len(opts) > listOptions {
				opts = opts[:listOptions]
			}
			if len(bl.Links) > 0 {
				b.WriteString("    links: " + linkRefs(bl.Links) + "\n")
			}
			for _, f := range bl.Attachments {
				b.WriteString("    file: " + f.Describe() + " (attachment_get)\n")
			}
			for j, o := range opts {
				fmt.Fprintf(&b, "    %d. %s (%s)", j+1, quoted(o.Answer, 160), n.short(o.By))
				if o.Recommended {
					b.WriteString(" [recommended]")
				}
				if o.Why != "" {
					b.WriteString(": " + quoted(o.Why, 160))
				}
				if o.Link != nil {
					b.WriteString(" link: " + linkRefs([]coord.Link{*o.Link}))
				}
				b.WriteString("\n")
			}
			if len(opts) < len(bl.Options) {
				fmt.Fprintf(&b, "    (+%d more answers)\n", len(bl.Options)-len(opts))
			}
		}
		if more > 0 {
			var ids []string
			for _, bl := range open[len(open)-more:] {
				ids = append(ids, oneLine(bl.ID, 20))
			}
			fmt.Fprintf(&b, "  +%d more open items (%s); show %s lists them all\n", more, strings.Join(ids, ", "), t.ShortRef())
		}
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
	v := s.boardView(repo, c.Sender.Key, doneLimit)
	asks, took := askGroups(&v, c.Sender.Key, repo)
	if repo != "" {
		// Asks from every repo: one with no repo is for anyone.
		all := s.b.Coord().Board(coord.BoardFilter{For: c.Sender.Key, DoneLimit: 1})
		asks, took = askGroups(&all, c.Sender.Key, repo)
	}
	where := "every repo"
	if repo != "" {
		where = oneLine(repo, 160)
	}
	open := len(asks) + len(took) + len(v.NeedsYou) + len(v.Blocked) + len(v.Working) + len(v.Open)
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
			line := s.cardText(cd.Task, cd.LastComment, n, now, true)
			if repo != "" && cd.RepoKey != repo {
				where := "no repo"
				if cd.RepoKey != "" {
					where = "repo " + oneLine(cd.RepoKey, 160)
				}
				line += "  from " + where + "\n"
			}
			if cd.Resolved {
				line += "  its blocker has gone away; it closes on the next sweep, or board_unblock now\n"
			}
			if len(cd.Gone) > 0 {
				line += "  what " + oneLine(strings.Join(cd.Gone, ", "), 80) + " waited on has gone away; the next sweep closes just those, the other items still wait\n"
			}
			if len(cd.Overlaps) > 0 {
				line += "  holders' leases overlap: " + oneLine(strings.Join(cd.Overlaps, "; "), maxFieldRunes) + "\n"
			}
			if b.Len()+len(line) > budget { // skip it, but later shorter cards still fit
				cut++
				continue
			}
			b.WriteString(line)
		}
	}
	group("Asks from the person, nobody has taken them (ask_claim one if you're free)", asks)
	group("Asks you took (ask_close each with what it became)", took)
	group("Needs you", v.NeedsYou)
	group("Blocked", v.Blocked)
	group("Working", v.Working)
	group("Open", v.Open)
	if len(v.Leases) > 0 && cut == 0 {
		fmt.Fprintf(&b, "\nLeases in the way (%d):\n", len(v.Leases))
		for _, l := range v.Leases {
			line := fmt.Sprintf("- %s held by %s, expires %s", oneLine(l.Resource, maxFieldRunes), n.who(l.Holder), in(now, l.Until))
			if len(l.Blocks) > 0 {
				var bs []string
				for _, id := range l.Blocks {
					bs = append(bs, s.shortOf(id))
				}
				line += "; blocks " + oneLine(strings.Join(bs, ", "), maxFieldRunes)
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
	b.WriteString("show #<n> (or task:<id>) for a card's comments and links.\n")
	return b.String()
}

// --- board_create ---

type boardCreateArgs struct {
	Title     string    `json:"title"`
	Objective string    `json:"objective"`
	DoneWhen  string    `json:"done_when"`
	Repo      string    `json:"repo"`
	Shared    bool      `json:"shared"`
	Links     []string  `json:"links"`
	AssignTo  string    `json:"assign_to"`
	Steps     []stepArg `json:"steps"`
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
		Steps: stepsOf(a.Steps),
	})
	if err != nil {
		return mapErr(err)
	}
	n := s.roster()
	out := "Created " + t.ShortRef() + " (task:" + t.ID + ")."
	if len(t.Steps) == 0 {
		out += " Give it steps with board_steps once you know them, so the person sees how far along it is."
	} else {
		out += " Tick its steps with board_step as you go."
	}
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
	if s.isOwner(v) {
		return v, nil
	}
	return s.resolveAgent(v)
}

// --- board_comment ---

type boardCommentArgs struct {
	TaskID    string   `json:"task_id"`
	Text      string   `json:"text"`
	Wake      bool     `json:"wake"`
	Attach    []string `json:"attach"`
	BlockerID string   `json:"blocker_id"`
	Missing   []string `json:"missing"`
}

func (s *set) boardComment(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardCommentArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
	}
	if strings.TrimSpace(a.Text) == "" {
		return errText("give the comment text.")
	}
	bid := strings.TrimSpace(a.BlockerID)
	if len(a.Missing) > 0 && bid == "" {
		return errText("missing needs blocker_id: which blocker's answer it is missing from.")
	}
	if len(a.Missing) > coord.MaxMissing {
		return errText(fmt.Sprintf("missing takes at most %d lines.", coord.MaxMissing))
	}
	files, msg := s.attachFiles(ctx, c, a.Attach)
	if msg != "" {
		return msg
	}
	cm, notice, err := s.b.Coord().CommentOn(ctx, c.Sender.Key, id, coord.CommentInput{
		Text: strings.TrimSpace(a.Text), Wake: a.Wake, Files: files, BlockerID: bid, Missing: a.Missing,
	})
	if err != nil {
		return mapErr(err)
	}
	if bid != "" {
		// A reply on a blocker tells nobody: the person is the audience,
		// and the item is back in their Needs you.
		b := oneLine(bid, 20)
		return "Replied on blocker " + b + " of " + s.shortOf(id) + attachedLine(files) + "; it stays open and goes back to the person. " +
			"Resolve it with board_unblock blocker_id=" + b + " note=… once you have what you need."
	}
	n := s.roster()
	out := "Commented comment:" + cm.ID + " on " + s.shortOf(cm.TaskID) + attachedLine(files) + "."
	if t, ok := s.b.Coord().GetTask(id); ok && t.Ask.Open() && t.Ask.Taker != c.Sender.Key && !slices.Contains(t.Watchers, c.Sender.Key) {
		// Chiming in on an ask makes you a watcher: you hear more only if
		// asked (board.md "Asks").
		if _, err := s.b.Coord().Watch(ctx, c.Sender.Key, id); err == nil {
			out += " You watch it now: you hear when it changes, and answer the person only if asked."
		}
	}
	if rs := RouteNotice(ctx, s.b.Inbox(), c.Sender, notice); len(rs) > 0 {
		out += s.told(rs, n)
	} else {
		out += " Nobody else holds it, so nobody was told; it shows on the Board."
	}
	return out
}

// --- board_block / board_unblock ---

type boardBlockArgs struct {
	TaskID   string `json:"task_id"`
	Question string `json:"question"`
	Reason   string `json:"reason"` // the old reason-only form: an alias of question
	Context  string `json:"context"`
	Options  []struct {
		Answer      string `json:"answer"`
		Why         string `json:"why"`
		Recommended bool   `json:"recommended"`
		Link        string `json:"link"`
	} `json:"options"`
	On     string   `json:"on"`
	Links  []string `json:"links"`
	Attach []string `json:"attach"`
}

// linkLabels is links' labels, comma separated.
func linkLabels(ls []coord.Link) string {
	var out []string
	for _, l := range ls {
		out = append(out, oneLine(l.Label, 60))
	}
	return strings.Join(out, ", ")
}

// linkRefs is links' refs (the URL or path itself), comma separated.
func linkRefs(ls []coord.Link) string {
	var out []string
	for _, l := range ls {
		out = append(out, oneLine(l.Ref, 300))
	}
	return strings.Join(out, ", ")
}

// manyQuestions reports text that asks more than one thing: two or more
// question marks. Each ask belongs in a blocker of its own.
func manyQuestions(text string) bool { return strings.Count(text, "?") > 1 }

const oneAskEach = "Raise each question with its own board_block (question, context for a newcomer, options when there are real choices, and recommended: true on the one you recommend), so the person can answer them one by one."

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
		if ref == "" {
			return nil, errText("on = task needs its short ref or id, e.g. task #12.")
		}
		var msg string
		if bb.Ref, msg = s.taskID(ref); msg != "" {
			return nil, msg
		}
		if _, unknown := coord.ParseShort(bb.Ref); unknown {
			return nil, errText(fmt.Sprintf("on = task: no card is %q; the board tool lists them.", oneLine(ref, 40)))
		}
	case coord.BlockedOnPerson:
		if ref != "" && !s.isOwner(ref) {
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
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
	}
	q, reason := strings.TrimSpace(a.Question), strings.TrimSpace(a.Reason)
	if q == "" && reason == "" {
		return errText("give question: the decision or thing in the way, one line, written for someone new to the work.")
	}
	if q != "" && reason != "" && q != reason {
		return errText("give question or reason, not both (reason is the old name for question).")
	}
	bb, msg := s.parseOn(a.On)
	if msg != "" {
		return msg
	}
	if len(a.Links) > coord.MaxBlockerLinks {
		return errText(fmt.Sprintf("links takes at most %d URLs or repo paths.", coord.MaxBlockerLinks))
	}
	for _, l := range a.Links {
		if strings.TrimSpace(l) == "" {
			continue
		}
		if _, err := coord.ParseLink(l); err != nil {
			return mapErr(err)
		}
	}
	for _, o := range a.Options {
		if strings.TrimSpace(o.Link) == "" {
			continue
		}
		if _, err := coord.ParseLink(o.Link); err != nil {
			return mapErr(err)
		}
	}
	files, msg := s.attachFiles(ctx, c, a.Attach)
	if msg != "" {
		return msg
	}
	co := s.b.Coord()
	var t coord.Task
	var item coord.Blocker
	var err error
	if q == "" && strings.TrimSpace(a.Context) == "" && len(a.Options) == 0 && len(a.Links) == 0 && len(files) == 0 {
		// The old reason-only form: one item that blocking again replaces.
		t, err = co.Block(ctx, c.Sender.Key, id, reason, bb)
		if err == nil {
			if open := t.OpenBlockers(); len(open) > 0 {
				item = open[0]
			}
		}
	} else {
		if q == "" {
			q = reason
		}
		var opts []coord.Option
		for _, o := range a.Options {
			opt := coord.Option{Answer: o.Answer, Why: o.Why, Recommended: o.Recommended}
			if strings.TrimSpace(o.Link) != "" {
				opt.Link = &coord.Link{Ref: o.Link}
			}
			opts = append(opts, opt)
		}
		if len(opts) == 1 {
			opts[0].Recommended = true // a lone answer is the one recommended
		}
		t, item, err = co.Raise(ctx, c.Sender.Key, id, coord.NewBlocker{Question: q, Context: a.Context, Options: opts, BlockedBy: bb,
			Links: a.Links, Files: files})
	}
	if err != nil {
		return mapErr(err)
	}
	out := "Blocked " + t.ShortRef()
	if item.ID != "" {
		out += " on blocker " + item.ID
	}
	out += attachedLine(files)
	if len(item.Links) > 0 {
		join := " with"
		if len(files) > 0 {
			join = " and"
		}
		out += join + " links " + linkLabels(item.Links)
	}
	out += "."
	switch {
	case len(item.Options) == 0:
		out += " Add your recommended answer with board_recommend so whoever decides can pick it."
	case !slices.ContainsFunc(item.Options, func(o coord.Option) bool { return o.Recommended }):
		out += " None of its options is marked recommended: board_recommend with the answer you recommend marks it."
	}
	if manyQuestions(item.Question) {
		out += " Its question asks more than one thing. " + oneAskEach
	}
	if bb != nil && (bb.Kind == coord.BlockedOnLease || bb.Kind == coord.BlockedOnTask) {
		out += " It unblocks by itself when that " + string(bb.Kind) + " goes away."
	} else {
		out += " Anyone may board_unblock it."
	}
	return out + "\n" + s.cardLine(t, nil, s.roster(), s.now())
}

type boardUnblockArgs struct {
	TaskID    string `json:"task_id"`
	Note      string `json:"note"`
	BlockerID string `json:"blocker_id"`
	Pick      int    `json:"pick"`
}

func (s *set) boardUnblock(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardUnblockArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
	}
	if bid := strings.TrimSpace(a.BlockerID); bid != "" {
		if a.Pick == 0 && strings.TrimSpace(a.Note) == "" {
			return errText("to resolve one blocker give pick (the number of a recommended answer) or note (your answer).")
		}
		t, notice, err := s.b.Coord().Answer(ctx, c.Sender.Key, id, bid, coord.BlockerAnswer{Pick: a.Pick, Text: a.Note, Resolve: true})
		if err != nil {
			return mapErr(err)
		}
		n := s.roster()
		out := "Resolved blocker " + oneLine(bid, 20) + " on " + t.ShortRef()
		if i := slices.IndexFunc(t.Blockers, func(b coord.Blocker) bool { return b.ID == bid }); i >= 0 &&
			t.Blockers[i].Resolved != nil && t.Blockers[i].Resolved.Note != "" {
			out += " with your note" // beside the person's answer
		}
		out += "; "
		if t.State == coord.Blocked {
			out += fmt.Sprintf("%d still open.", len(t.OpenBlockers()))
		} else {
			out += "it is " + string(t.State) + " again."
		}
		return out + s.told(RouteNotice(ctx, s.b.Inbox(), c.Sender, notice), n) + "\n" + s.cardLine(t, nil, n, s.now())
	}
	if a.Pick != 0 {
		return errText("pick needs blocker_id: which blocker it answers.")
	}
	t, notice, err := s.b.Coord().Unblock(ctx, c.Sender.Key, id, strings.TrimSpace(a.Note))
	if err != nil {
		return mapErr(err)
	}
	n := s.roster()
	return "Unblocked " + t.ShortRef() + "; it is " + string(t.State) + " again." +
		s.told(RouteNotice(ctx, s.b.Inbox(), c.Sender, notice), n) + "\n" + s.cardLine(t, nil, n, s.now())
}

// --- board_recommend ---

type boardRecommendArgs struct {
	TaskID    string `json:"task_id"`
	BlockerID string `json:"blocker_id"`
	Answer    string `json:"answer"`
	Why       string `json:"why"`
}

func (s *set) boardRecommend(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardRecommendArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
	}
	if strings.TrimSpace(a.BlockerID) == "" {
		return errText("give blocker_id: the blocker's id as the board tool shows it ([b1]).")
	}
	if strings.TrimSpace(a.Answer) == "" {
		return errText("give answer: the answer you recommend, one line.")
	}
	t, err := s.b.Coord().Recommend(ctx, c.Sender.Key, id, strings.TrimSpace(a.BlockerID), a.Answer, a.Why)
	if err != nil {
		return mapErr(err)
	}
	return "Recommended on blocker " + oneLine(a.BlockerID, 20) + " of " + t.ShortRef() + ". Whoever decides sees it beside the question.\n" +
		s.cardLine(t, nil, s.roster(), s.now())
}

// --- board_steps / board_step ---

// stepArg is one step: an object, or just its text.
type stepArg struct {
	ID    string `json:"id"`
	Text  string `json:"text"`
	Done  bool   `json:"done"`
	Doing bool   `json:"doing"`
}

func (st *stepArg) UnmarshalJSON(b []byte) error {
	var text string
	if json.Unmarshal(b, &text) == nil {
		*st = stepArg{Text: text}
		return nil
	}
	type plain stepArg
	var p plain
	if err := json.Unmarshal(b, &p); err != nil {
		return err
	}
	*st = stepArg(p)
	return nil
}

// stepsOf turns step arguments into coord's steps.
func stepsOf(in []stepArg) []coord.Step {
	steps := make([]coord.Step, 0, len(in))
	for _, st := range in {
		steps = append(steps, coord.Step{ID: st.ID, Text: st.Text, Done: st.Done, Doing: st.Doing})
	}
	return steps
}

type boardStepsArgs struct {
	TaskID string    `json:"task_id"`
	Steps  []stepArg `json:"steps"`
}

func (s *set) boardSteps(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardStepsArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
	}
	t, err := s.b.Coord().SetSteps(ctx, c.Sender.Key, id, stepsOf(a.Steps))
	if err != nil {
		return mapErr(err)
	}
	var b strings.Builder
	d, n := t.Progress()
	fmt.Fprintf(&b, "%s has %d steps, %d done. Tick them with board_step as you go.\n", t.ShortRef(), n, d)
	for _, st := range t.Steps {
		mark := "[ ]"
		switch {
		case st.Done:
			mark = "[x]"
		case st.Doing:
			mark = "[>]"
		}
		fmt.Fprintf(&b, "- %s %s %s\n", mark, oneLine(st.ID, 20), quoted(st.Text, maxFieldRunes))
	}
	return b.String()
}

type boardStepArgs struct {
	TaskID string `json:"task_id"`
	StepID string `json:"step_id"`
	Done   *bool  `json:"done"`
	Doing  *bool  `json:"doing"`
}

func (s *set) boardStep(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a boardStepArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
	}
	if strings.TrimSpace(a.StepID) == "" {
		return errText("give step_id: the step's id as board_steps or the board tool shows it (s1).")
	}
	if a.Done == nil && a.Doing == nil {
		return errText("give done (true ticks it, false unticks it) or doing (true makes it the current step).")
	}
	t, err := s.b.Coord().SetStep(ctx, c.Sender.Key, id, strings.TrimSpace(a.StepID), coord.StepUpdate{Done: a.Done, Doing: a.Doing})
	if err != nil {
		return mapErr(err)
	}
	d, n := t.Progress()
	return fmt.Sprintf("%s: steps %d/%d done.\n", t.ShortRef(), d, n) + s.cardLine(t, nil, s.roster(), s.now())
}

// --- board_watch / board_unwatch ---

func (s *set) boardWatch(ctx context.Context, c Caller, raw json.RawMessage) string {
	return s.boardWatching(ctx, c, raw, true)
}

func (s *set) boardUnwatch(ctx context.Context, c Caller, raw json.RawMessage) string {
	return s.boardWatching(ctx, c, raw, false)
}

func (s *set) boardWatching(ctx context.Context, c Caller, raw json.RawMessage, on bool) string {
	var a boardTaskArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
	}
	co := s.b.Coord()
	var t coord.Task
	var err error
	if on {
		t, err = co.Watch(ctx, c.Sender.Key, id)
	} else {
		t, err = co.Unwatch(ctx, c.Sender.Key, id)
	}
	if err != nil {
		return mapErr(err)
	}
	out := "You watch " + t.ShortRef() + " now: you hear at your next turn when its state, steps, blockers or PR links change."
	if !on {
		out = "You no longer watch " + t.ShortRef() + "."
	}
	return out + "\n" + s.cardLine(t, nil, s.roster(), s.now())
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
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
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
	out := "You hold " + t.ShortRef() + " now"
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
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
	}
	t, err := s.b.Coord().Leave(ctx, c.Sender.Key, id, c.Sender.Key)
	if err != nil {
		return mapErr(err)
	}
	if t.Ask.Open() {
		return "You gave the ask " + t.ShortRef() + " back; the device that hands it out tells the room again."
	}
	return "You left " + t.ShortRef() + "; it stays on the Board.\n" + s.cardLine(t, nil, s.roster(), s.now())
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
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
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
	return "Assigned " + t.ShortRef() + " to " + n.who(to) + "." + s.told(RouteNotice(ctx, s.b.Inbox(), c.Sender, notice), n) +
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
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
	}
	st := coord.TaskState(strings.ToLower(strings.TrimSpace(a.State)))
	switch {
	case st == coord.Blocked:
		return errText("use board_block for blocked: it takes the reason.")
	case !slices.Contains(boardStates, st):
		return errText("give state: working, input_required, completed, failed or canceled.")
	case st == coord.InputRequired:
		return s.inputRequired(id, strings.TrimSpace(a.Note))
	}
	t, err := s.b.Coord().SetTaskState(ctx, c.Sender.Key, id, st, strings.TrimSpace(a.Note))
	if err != nil {
		return mapErr(err)
	}
	out := t.ShortRef() + " is " + string(t.State) + " now."
	if coord.IsTerminal(t.State) {
		out += " It folds into Done on the Board."
	}
	return out + "\n" + s.cardLine(t, nil, s.roster(), s.now())
}

// inputRequired answers board_status input_required. What the person must
// decide goes on the card as blockers, one per question, so the window can
// show each with its options and the agent's recommendation; a note holding
// several asks showed as one fat card. A card with no open blocker is
// refused with how to raise them. A card that has them is already Blocked
// and in the person's Needs you, so nothing changes: moving it to
// input_required would close every open blocker (a state change does).
func (s *set) inputRequired(id, note string) string {
	t, ok := s.b.Coord().GetTask(id)
	if !ok {
		return mapErr(fmt.Errorf("%w: task %s", coord.ErrNotFound, oneLine(id, 80)))
	}
	open := t.OpenBlockers()
	if len(open) == 0 {
		msg := "a card needs your question on it as a blocker before it waits on the person: input_required alone shows them one note with nothing to pick. " + oneAskEach
		if manyQuestions(note) {
			msg = "your note asks more than one thing; " + msg
		}
		return errText(msg)
	}
	var ids []string
	for _, b := range open {
		ids = append(ids, oneLine(b.ID, 20))
	}
	out := fmt.Sprintf("%s already waits on %d open blockers (%s); they are in the person's window, each answered on its own, so its state stays %s.",
		t.ShortRef(), len(open), strings.Join(ids, ", "), t.State)
	if note != "" {
		out += " The note was not saved: put anything else the person needs to decide in its own board_block, or a board_comment."
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
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
	}
	if strings.TrimSpace(a.Link) == "" {
		return errText("give url_or_path: an http(s) URL or a repo path.")
	}
	t, err := s.b.Coord().AddLink(ctx, c.Sender.Key, id, a.Link)
	if err != nil {
		return mapErr(err)
	}
	return "Linked " + t.ShortRef() + ".\n" + s.cardLine(t, nil, s.roster(), s.now())
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

package notify

import (
	"context"
	"fmt"
	"log"
	"slices"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/StephenSHorton/irori/internal/ask"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
)

// InboxSource is the part of *inbox.Inbox Watch uses.
type InboxSource interface {
	Subscribe(ctx context.Context) <-chan inbox.Change
	Get(deliveryID string) (inbox.Delivery, bool)
	Open() []inbox.Delivery
	Release(ctx context.Context, deliveryID, by string) error
	Dismiss(ctx context.Context, deliveryID, by string) error
}

// BoardSource is the part of *coord.Coord Watch uses.
type BoardSource interface {
	Subscribe(ctx context.Context) <-chan coord.Change
	Board(f coord.BoardFilter) coord.BoardView
}

// AskSource is the part of *ask.Book Watch uses.
type AskSource interface {
	Subscribe(ctx context.Context) <-chan string
	Get(id string) (ask.Ask, bool)
	Open() []ask.Ask
	Answer(ctx context.Context, id, choice, by string) error
}

// RoomSource is the part of *coord.Coord the unclaimed-ask card acts
// through (board.md "Asks"): asking the room again, and closing it.
type RoomSource interface {
	AskAgain(ctx context.Context, by, id string) error
	SetTaskState(ctx context.Context, by, id string, s coord.TaskState, note string) (coord.Task, error)
}

// Sources are what Watch follows and acts through.
type Sources struct {
	Inbox InboxSource
	Board BoardSource
	// Asks are the person's approvals as rows (v0.4 §6.7): every desktop
	// device shows each open one, any of them answers, and the card clears
	// everywhere once the asking device resolves it.
	Asks AskSource
	// User is the person at this device ("user:<device>"): Release and
	// Dismiss act as them, and the Board's "needs you" means them.
	User string
	// IsOwner reports whether a key is the person's on any of their devices
	// (v0.4 identity.Chain.IsOwner): a message to the person on another
	// device, or a card blocked on them there, is theirs here too. Nil
	// matches exactly User.
	IsOwner func(key string) bool
	// Name shows an agent key as its display name; nil shows the key.
	Name func(key string) string
	// Room, when set, posts a card for an ask nobody took (unclaimed:<id>)
	// with Wake them and Close, on every desktop device; it clears once
	// someone takes the ask or it closes.
	Room RoomSource
}

// Tabs a card opens.
const (
	TabInbox = "inbox"
	TabBoard = "board"
)

// Watch posts and clears cards for what needs the person, driven only by
// the inbox and coord change streams (no polling), until ctx ends:
//
//   - held deliveries (not ones held by Pause all, which the person chose),
//     one card per sender (decision 34): "<sender> sent <agent> a message
//     Irori held", or "<sender> sent <agent> 3 messages Irori held" once
//     more are held, with Release and Dismiss (Release all, Dismiss all)
//     acting through the inbox as the person on every one the card counts
//     (a press that leaves any held puts the card back quietly, with the
//     error and its buttons; one resolved elsewhere meanwhile is no error).
//     A new hold from that sender updates the card in place, silently
//     while it is up (only a card going up sounds); a hold that leaves
//     Held updates an open card and clears it with the last. Closing the
//     card with its x leaves the messages held, and the card comes back,
//     silently, on the next inbox change (the window has no inbox list);
//   - a Board card that an agent (not the person) moved into the person's
//     Needs you lane (blocked on them, waiting on their input, or assigned
//     to them): Answer opens the Board at the card, Later closes it;
//     cleared when it leaves the lane;
//   - a message addressed to the person (User): Open shows it in the
//     window (to read in full and reply); cleared once it is no longer
//     waiting.
//
// Held messages and messages to the person already waiting when Watch
// starts get their cards then: the cards are the only way to them. A Board
// card already in Needs you doesn't pop up, since the window shows it. A
// card body click opens the window at the card's target (Notifier.Open).
//
// Both streams drop changes for a slow reader (the inbox silently, coord
// with a Resync), so neither is taken as complete: the Board is reconciled
// on every coord change, and the inbox against a fresh snapshot of its open
// deliveries each time the inbox stream has been drained.
func Watch(ctx context.Context, n *Notifier, s Sources) {
	if n.Headless() {
		return
	}
	w := &watcher{n: n, s: s, held: map[string]string{}, groups: map[string]*heldGroup{},
		dirty: map[string]heldChange{}, closed: map[string]bool{},
		msgs: map[string]bool{}, needs: map[string]bool{},
		asks: map[string]bool{}, unclaimed: map[string]bool{}, forget: make(chan string, 16),
		failed: make(chan heldFail, 16), fails: map[string]string{}}
	var ich <-chan inbox.Change
	var cch <-chan coord.Change
	var ach <-chan string
	if s.Asks != nil {
		ach = s.Asks.Subscribe(ctx)
		for _, a := range s.Asks.Open() {
			w.ask(ctx, a.ID) // waiting asks get their card: it is the only way to them
		}
	}
	if s.Inbox != nil {
		ich = s.Inbox.Subscribe(ctx)
		w.reconcile(ctx, true) // what waits from before start gets its card
	}
	if s.Board != nil {
		cch = s.Board.Subscribe(ctx)
		w.board(false) // seed: what is already waiting stays quiet
	}
	for ich != nil || cch != nil || ach != nil {
		select {
		case <-ctx.Done():
			return
		case id, ok := <-ach:
			if !ok {
				ach = nil
				continue
			}
			w.ask(ctx, id)
		case c, ok := <-ich:
			if !ok {
				ich = nil
				continue
			}
			w.inbox(ctx, c)
			if len(ich) == 0 {
				w.reconcile(ctx, true)
			}
		case _, ok := <-cch:
			if !ok {
				cch = nil
				continue
			}
			w.board(true)
		case f := <-w.failed:
			// A press took the card down. Re-read what it counted, and put
			// it back up quietly for whatever is still held (one the press
			// couldn't release, or one held after the card was built), with
			// the error, buttons and all, until the next press.
			if g := w.groups[f.key]; g != nil {
				for _, id := range slices.Clone(g.ids) {
					if d, ok := w.s.Inbox.Get(id); ok {
						w.delivery(ctx, d, false, false)
					} else {
						w.unhold(id)
					}
				}
			}
			if f.err == "" {
				delete(w.fails, f.key)
			} else {
				w.fails[f.key] = f.err
			}
			w.dirty[f.key] = max(w.dirty[f.key], postQuiet)
			w.flush(ctx)
		case from := <-w.forget:
			// Its card was closed with the x: the next reconcile posts it
			// again, without a sound for the holds it already showed.
			if g := w.groups[from]; g != nil {
				for _, id := range g.ids {
					delete(w.held, id)
					w.closed[id] = true
				}
				delete(w.groups, from)
			}
		}
	}
}

type watcher struct {
	n *Notifier
	s Sources
	// held maps each held delivery id accounted for (on a card, or seeded)
	// to the sender whose card counts it; groups are those cards, by sender
	// key. dirty marks the senders whose card changed since the last flush
	// and how, closed the delivery ids whose card the person closed with
	// its x (they come back without a sound).
	held   map[string]string
	groups map[string]*heldGroup
	dirty  map[string]heldChange
	closed map[string]bool
	msgs   map[string]bool // delivery ids to the person accounted for (likewise)
	needs  map[string]bool // task ids in Needs you (card posted, or seeded)
	asks   map[string]bool // ask ids with a card up
	// unclaimed are the Board asks nobody took (card posted, or seeded).
	unclaimed map[string]bool
	// forget carries the sender keys whose held card the person closed
	// (from the card's own goroutine to Watch's loop), failed the outcome
	// of each Release or Dismiss press; fails is the error a sender's card
	// shows until the next press or until nothing of theirs is held.
	forget chan string
	failed chan heldFail
	fails  map[string]string
}

// heldFail is one press of a held card's Release or Dismiss: the error to
// show on the sender's card, "" when every message it acted on went.
type heldFail struct{ key, err string }

// heldGroup is one sender's held deliveries, oldest first.
type heldGroup struct {
	ids  []string
	dels map[string]inbox.Delivery
}

// heldChange is what a flush does with a sender's held card, the strongest
// change since the last flush winning.
type heldChange int

const (
	updateIfUp heldChange = iota // a hold left: update the card if it is up, clear it with the last
	postQuiet                    // put the card up (again) without a sound
	postLoud                     // a new hold: put the card up (with a sound) or update it
)

// unhold drops a held delivery from its sender's card.
func (w *watcher) unhold(id string) {
	key, ok := w.held[id]
	if !ok {
		return
	}
	delete(w.held, id)
	if g := w.groups[key]; g != nil {
		g.ids = slices.DeleteFunc(g.ids, func(s string) bool { return s == id })
		delete(g.dels, id)
	}
	w.dirty[key] = max(w.dirty[key], updateIfUp)
}

// flush brings each changed sender's held card in line: cleared when none
// of theirs is held any more, else posted or updated in place per
// heldChange. Only a card going up for a new hold makes a sound; updating
// one that is up never does.
func (w *watcher) flush(ctx context.Context) {
	for key, c := range w.dirty {
		delete(w.dirty, key)
		id := HeldID(key)
		g := w.groups[key]
		if g == nil || len(g.ids) == 0 {
			delete(w.groups, key)
			delete(w.fails, key)
			w.n.Clear(id)
			continue
		}
		if c == updateIfUp && !w.n.Live(id) {
			continue
		}
		note := w.heldNote(ctx, key, g)
		note.Silent = c != postLoud || w.n.Live(id)
		w.n.Post(note)
	}
}

func (w *watcher) name(key string) string {
	if w.s.Name != nil {
		if s := strings.TrimSpace(w.s.Name(key)); s != "" {
			return s
		}
	}
	if key == "" {
		return "someone"
	}
	return key
}

// inbox handles one inbox change. Only changes that name a delivery matter.
func (w *watcher) inbox(ctx context.Context, c inbox.Change) {
	if c.DeliveryID == "" {
		return
	}
	d, ok := w.s.Inbox.Get(c.DeliveryID)
	if !ok {
		return
	}
	w.delivery(ctx, d, c.Kind == inbox.HeldNew, true)
	w.flush(ctx)
}

// ask brings one ask's card in line: posted while it waits for an answer,
// cleared once it is resolved (on any device's answer, or at its end).
func (w *watcher) ask(ctx context.Context, id string) {
	a, ok := w.s.Asks.Get(id)
	switch {
	case !ok:
		return
	case a.Resolved:
		if w.asks[id] {
			delete(w.asks, id)
			w.n.Clear(AskID(id))
		}
		return
	case w.asks[id]:
		return
	}
	left := time.Until(a.Until)
	if left <= 0 {
		return
	}
	w.asks[id] = true
	title := strings.TrimSpace(a.Name)
	if title == "" {
		title = w.name(a.By)
	}
	if a.Title != "" {
		title += ": " + a.Title
	}
	level, _ := ParseLevel(a.Level)
	note := Note{ID: AskID(id), Title: title, Body: a.Text, Level: level, Expire: left}
	user := w.s.User
	answer := func(choice string) {
		if err := w.s.Asks.Answer(ctx, id, choice, user); err != nil && ctx.Err() == nil {
			log.Printf("irori: answer %s: %v", id, err)
		}
	}
	for _, l := range a.Actions {
		note.Actions = append(note.Actions, Action{Label: l, Run: func() { answer(l) }})
	}
	note.OnDismiss = func() { answer("") }
	w.n.Post(note)
}

// cardHeld is a hold the person is asked about (Pause all is their own).
// stillHeld reports whether delivery id is still held: a Release or
// Dismiss that failed on one released, dismissed or gone elsewhere since
// the card went up is no failure; one that left it held (a repo with no
// agent to take it yet) is.
func (w *watcher) stillHeld(id string) bool {
	d, ok := w.s.Inbox.Get(id)
	return ok && d.State == inbox.Held
}

func cardHeld(d inbox.Delivery) bool { return d.State == inbox.Held && d.Hold != inbox.HoldPaused }

// owner reports whether key is the person's (Sources.IsOwner, else User).
func (w *watcher) owner(key string) bool {
	if w.s.IsOwner != nil {
		return w.s.IsOwner(key)
	}
	return w.s.User != "" && key == w.s.User
}

// waitingFor is a message to the person they haven't seen yet.
func (w *watcher) waitingFor(d inbox.Delivery) bool {
	return w.owner(d.To) && (d.State == inbox.Queued || d.State == inbox.Delivered)
}

// delivery brings d's cards in line with its state: a hold or a message
// new to the watcher gets its card (only when post), and a card is taken
// down or updated once it no longer applies. heldAgain counts a hold
// already carded as new (it was held anew, so its reason may differ).
// Held cards change only in the next flush.
func (w *watcher) delivery(ctx context.Context, d inbox.Delivery, heldAgain, post bool) {
	id := d.ID
	from, accounted := w.held[id]
	switch {
	case cardHeld(d):
		key := d.Message.From.Key
		if accounted && from != key {
			w.unhold(id) // can't happen (a message keeps its sender); stay consistent
			accounted = false
		}
		g := w.groups[key]
		if g == nil {
			g = &heldGroup{dels: map[string]inbox.Delivery{}}
			w.groups[key] = g
		}
		if !accounted {
			g.ids = append(g.ids, id)
		}
		prev := g.dels[id]
		g.dels[id] = d
		w.held[id] = key
		if accounted && heldAgain && prev.Hold == d.Hold && w.n.Live(HeldID(key)) {
			heldAgain = false // the card already shows it, for the same reason
		}
		if (!accounted || heldAgain) && post {
			c := postLoud
			if w.closed[id] && !heldAgain {
				c = postQuiet // its card was closed with the x: back, without a sound
			}
			w.dirty[key] = max(w.dirty[key], c)
		}
		delete(w.closed, id)
	case accounted:
		w.unhold(id)
	}
	switch waiting := w.waitingFor(d); {
	case waiting && !w.msgs[id]:
		w.msgs[id] = true
		if post {
			w.n.Post(w.messageNote(d))
		}
	case !waiting && w.msgs[id]:
		delete(w.msgs, id)
		w.n.Clear(MessageID(id))
	}
}

// reconcile re-reads the inbox's open deliveries, so a change the stream
// dropped (it drops for a full buffer and doesn't say) can't leave a card up
// for something resolved, or leave a new hold without one. post false seeds:
// what is already waiting is accounted for without a card.
func (w *watcher) reconcile(ctx context.Context, post bool) {
	open := map[string]bool{}
	for _, d := range w.s.Inbox.Open() {
		open[d.ID] = true
		w.delivery(ctx, d, false, post)
	}
	// What left the open set (dismissed, done, gone) is no longer held or
	// waiting; re-read it to be sure, and take its cards down.
	var left []string
	for id := range w.held {
		if !open[id] {
			left = append(left, id)
		}
	}
	for id := range w.msgs {
		if !open[id] {
			left = append(left, id)
		}
	}
	for id := range w.closed {
		if !open[id] {
			delete(w.closed, id) // resolved while its card was closed
		}
	}
	for _, id := range left {
		if d, ok := w.s.Inbox.Get(id); ok {
			w.delivery(ctx, d, false, post)
			continue
		}
		w.unhold(id)
		if w.msgs[id] {
			delete(w.msgs, id)
			w.n.Clear(MessageID(id))
		}
	}
	w.flush(ctx)
}

// holdReasons say why Irori held a delivery, in the card's words.
var holdReasons = map[inbox.HoldReason]string{
	inbox.HoldBlocked:        "the sender is on your block list",
	inbox.HoldNotAllowed:     "the sender isn't on your allow list",
	inbox.HoldBypassReceiver: "the recipient runs without permission prompts",
	inbox.HoldHopLimit:       "it is too many hops down a chain of agent messages",
	inbox.HoldUnowned:        "no agent in that repo can take it yet",
	inbox.HoldPaused:         "wakes are paused",
}

// HoldReason is a hold in a person's words.
func HoldReason(h inbox.HoldReason) string {
	if s, ok := holdReasons[h]; ok {
		return s
	}
	return string(h)
}

// heldTo is who a held delivery was for, in the card's words.
func (w *watcher) heldTo(d inbox.Delivery) string {
	switch {
	case d.To == "" && d.RepoKey != "":
		return "repo " + d.RepoKey
	case d.To != w.s.User:
		return w.name(d.To)
	}
	return "you"
}

// heldNote is sender key's held card: every delivery of theirs g holds,
// the newest one's reason and text on it.
func (w *watcher) heldNote(ctx context.Context, key string, g *heldGroup) Note {
	user := w.s.User
	ids := slices.Clone(g.ids)
	// The newest by id (ids are minted sortable), whatever order the
	// watcher saw them in.
	last := g.dels[slices.MaxFunc(ids, func(a, b string) int {
		return strings.Compare(g.dels[a].Message.ID+a, g.dels[b].Message.ID+b)
	})]
	from := last.Message.From.Name
	if from == "" {
		from = w.name(key)
	}
	to := w.heldTo(last)
	for _, id := range ids {
		if w.heldTo(g.dels[id]) != to {
			to = ""
			break
		}
	}
	var title string
	switch n := len(ids); {
	case n == 1:
		title = fmt.Sprintf("%s sent %s a message Irori held", from, to)
	case to == "":
		title = fmt.Sprintf("%s sent %d messages Irori held", from, n)
	default:
		title = fmt.Sprintf("%s sent %s %d messages Irori held", from, to, n)
	}
	what, release, dismiss := "that message", "Release", "Dismiss"
	if len(ids) > 1 {
		what, release, dismiss = "those messages", "Release all", "Dismiss all"
	}
	act := func(verb string, fn func(context.Context, string, string) error) func() {
		return func() {
			var errs []error
			for _, id := range ids {
				err := fn(ctx, id, user)
				if err != nil && w.stillHeld(id) {
					log.Printf("irori: notifications: %s %s: %v", verb, id, err)
					errs = append(errs, err)
				}
			}
			f := heldFail{key: key}
			if len(errs) > 0 {
				f.err = "Couldn't " + verb + " " + what + ": " + errs[0].Error()
			}
			select {
			case w.failed <- f:
			default: // full: the messages stay held either way
			}
		}
	}
	open := Target{Tab: TabInbox, Ref: last.ID}
	body, level := upper(HoldReason(last.Hold))+": "+preview(last.Message.Text), Warn
	if e := w.fails[key]; e != "" {
		body, level = e+". "+body, Error
	}
	return Note{
		ID:     HeldID(key),
		Title:  title,
		Body:   body,
		Level:  level,
		Sticky: true,
		Actions: []Action{
			{Label: release, Run: act("release", w.s.Inbox.Release)},
			{Label: dismiss, Run: act("dismiss", w.s.Inbox.Dismiss)},
		},
		OnClick: func() { w.n.Open(open) },
		OnDismiss: func() {
			select {
			case w.forget <- key:
			default: // full: they stay held, and the card is back after a restart
			}
		},
	}
}

func (w *watcher) messageNote(d inbox.Delivery) Note {
	from := d.Message.From.Name
	if from == "" {
		from = w.name(d.Message.From.Key)
	}
	open := Target{Tab: TabInbox, Ref: d.ID}
	return Note{
		ID:      MessageID(d.ID),
		Title:   from + " sent you a message",
		Body:    preview(d.Message.Text),
		Level:   Info,
		Actions: []Action{{Label: "Open", Run: func() { w.n.Open(open) }}},
		OnClick: func() { w.n.Open(open) },
	}
}

// board reconciles the Needs you lane with the cards: a task that entered
// it gets a card (when post), one that left has its card cleared.
func (w *watcher) board(post bool) {
	v := w.s.Board.Board(coord.BoardFilter{For: w.s.User, DoneLimit: 1})
	now := map[string]bool{}
	for _, c := range v.NeedsYou {
		now[c.ID] = true
		if w.needs[c.ID] {
			continue
		}
		w.needs[c.ID] = true
		if post && w.agentMoved(c.Task) {
			w.n.Post(w.needsNote(c))
		}
	}
	for id := range w.needs {
		if !now[id] {
			delete(w.needs, id)
			w.n.Clear(NeedsID(id))
		}
	}
	if w.s.Room != nil {
		w.unclaimedAsks(v, post)
	}
}

// unclaimedAsks posts a card for each open ask nobody took within
// coord.AskUnclaimed, and clears it once it is taken or closed.
func (w *watcher) unclaimedAsks(v coord.BoardView, post bool) {
	now := map[string]bool{}
	for _, g := range [][]coord.Card{v.NeedsYou, v.Blocked, v.Working, v.Open} {
		for _, c := range g {
			a := c.Ask
			if !a.Open() || !a.Unclaimed || a.Taker != "" {
				continue
			}
			now[c.ID] = true
			if w.unclaimed[c.ID] {
				continue
			}
			w.unclaimed[c.ID] = true
			if post {
				w.n.Post(w.unclaimedNote(c.Task))
			}
		}
	}
	for id := range w.unclaimed {
		if !now[id] {
			delete(w.unclaimed, id)
			w.n.Clear(UnclaimedID(id))
		}
	}
}

func (w *watcher) unclaimedNote(t coord.Task) Note {
	title := "Nobody took “" + preview(t.Title) + "”"
	if t.Short != "" {
		title += " (#" + t.Short + ")"
	}
	body := "Nobody was free to hear it."
	if t.Ask.Told > 0 {
		body = fmt.Sprintf("%d agents heard it at their next turn; none took it.", t.Ask.Told)
	}
	open := Target{Tab: TabBoard, Ref: "task:" + t.ID}
	id := t.ID
	return Note{
		ID:     UnclaimedID(t.ID),
		Title:  title,
		Body:   body,
		Level:  Warn,
		Sticky: true,
		Actions: []Action{
			{Label: "Wake them", Run: func() {
				if err := w.s.Room.AskAgain(context.Background(), w.s.User, id); err != nil {
					log.Printf("notify: ask again %s: %v", id, err)
				}
			}},
			{Label: "Close", Run: func() {
				if _, err := w.s.Room.SetTaskState(context.Background(), w.s.User, id, coord.Canceled, "closed by the person"); err != nil {
					log.Printf("notify: close ask %s: %v", id, err)
				}
			}},
		},
		OnClick: func() { w.n.Open(open) },
	}
}

// agentMoved says whether t's move into Needs you is news to the person:
// not their own doing (they claimed it, assigned it to themselves, blocked
// it on themselves), and about someone besides them.
func (w *watcher) agentMoved(t coord.Task) bool {
	if w.owner(t.UpdatedBy) {
		return false
	}
	return w.needsKey(t) != ""
}

// needsKey is who a Needs you card is about: its first holder that isn't
// the person, else its creator unless that is the person; "" for neither.
func (w *watcher) needsKey(t coord.Task) string {
	for _, h := range t.Holders {
		if !w.owner(h) {
			return h
		}
	}
	if !w.owner(t.CreatedBy) {
		return t.CreatedBy
	}
	return ""
}

// needsAgent is needsKey's display name.
func (w *watcher) needsAgent(t coord.Task) string { return w.name(w.needsKey(t)) }

func (w *watcher) needsNote(c coord.Card) Note {
	t := c.Task
	agent := w.needsAgent(t)
	var title, body string
	switch {
	case t.State == coord.Blocked:
		title = agent + " is blocked on you"
		body = t.Reason
		if t.Title != "" && t.Title != t.Reason {
			body += " · " + t.Title
		}
	case t.State == coord.InputRequired:
		title, body = agent+" needs your input", t.Title
		if t.Note != "" {
			body += " · " + t.Note
		}
	case slices.ContainsFunc(t.Holders, w.owner):
		title, body = agent+" handed you a card", t.Title
	default:
		title, body = agent+" needs you", t.Title
	}
	if t.Short != "" {
		title += " (#" + t.Short + ")" // what the person says to an agent about it
	}
	open := Target{Tab: TabBoard, Ref: "task:" + t.ID}
	return Note{
		ID:     NeedsID(t.ID),
		Title:  title,
		Body:   preview(body),
		Level:  Warn,
		Sticky: true,
		Actions: []Action{
			{Label: "Answer", Run: func() { w.n.Open(open) }},
			{Label: "Later"},
		},
		OnClick: func() { w.n.Open(open) },
	}
}

// preview is text on one line, cut to what two card lines can hold.
func preview(s string) string {
	s = strings.Join(strings.Fields(s), " ")
	const max = 160
	if utf8.RuneCountInString(s) > max {
		r := []rune(s)
		s = strings.TrimSpace(string(r[:max-1])) + "…"
	}
	return s
}

func upper(s string) string {
	if s == "" {
		return s
	}
	r, n := utf8.DecodeRuneInString(s)
	return strings.ToUpper(string(r)) + s[n:]
}

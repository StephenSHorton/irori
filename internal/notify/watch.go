package notify

import (
	"context"
	"fmt"
	"log"
	"slices"
	"strings"
	"unicode/utf8"

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

// Sources are what Watch follows and acts through.
type Sources struct {
	Inbox InboxSource
	Board BoardSource
	// User is the person at this device ("user:<device>"): Release and
	// Dismiss act as them, and the Board's "needs you" means them.
	User string
	// Name shows an agent key as its display name; nil shows the key.
	Name func(key string) string
}

// Tabs a card opens.
const (
	TabInbox = "inbox"
	TabBoard = "board"
)

// Watch posts and clears cards for what needs the person, driven only by
// the inbox and coord change streams (no polling), until ctx ends:
//
//   - a held delivery (not one held by Pause all, which the person chose):
//     "<sender> sent <agent> a message Irori held" with Release and Dismiss,
//     acting through the inbox as the person; cleared when it leaves Held.
//     Closing the card with its x leaves the message held, and the card
//     comes back on the next inbox change (the window has no inbox list);
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
	w := &watcher{n: n, s: s, held: map[string]bool{}, msgs: map[string]bool{}, needs: map[string]bool{},
		forget: make(chan string, 16)}
	var ich <-chan inbox.Change
	var cch <-chan coord.Change
	if s.Inbox != nil {
		ich = s.Inbox.Subscribe(ctx)
		w.reconcile(ctx, true) // what waits from before start gets its card
	}
	if s.Board != nil {
		cch = s.Board.Subscribe(ctx)
		w.board(false) // seed: what is already waiting stays quiet
	}
	for ich != nil || cch != nil {
		select {
		case <-ctx.Done():
			return
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
		case id := <-w.forget:
			// Its card was closed with the x: the next reconcile posts it again.
			delete(w.held, id)
		}
	}
}

type watcher struct {
	n     *Notifier
	s     Sources
	held  map[string]bool // held delivery ids accounted for (card posted, or seeded)
	msgs  map[string]bool // delivery ids to the person accounted for (likewise)
	needs map[string]bool // task ids in Needs you (card posted, or seeded)
	// forget carries held delivery ids whose card the person closed (from
	// the card's own goroutine to Watch's loop).
	forget chan string
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
}

// cardHeld is a hold the person is asked about (Pause all is their own).
func cardHeld(d inbox.Delivery) bool { return d.State == inbox.Held && d.Hold != inbox.HoldPaused }

// waitingFor is a message to the person they haven't seen yet.
func (w *watcher) waitingFor(d inbox.Delivery) bool {
	return w.s.User != "" && d.To == w.s.User && (d.State == inbox.Queued || d.State == inbox.Delivered)
}

// delivery brings d's cards in line with its state: a card for a hold or a
// message new to the watcher (posted only when post), the card taken down
// once it no longer applies. heldAgain reposts a hold already carded (it
// was held anew, so its reason may differ).
func (w *watcher) delivery(ctx context.Context, d inbox.Delivery, heldAgain, post bool) {
	id := d.ID
	switch {
	case cardHeld(d):
		if !w.held[id] || heldAgain {
			w.held[id] = true
			if post {
				w.n.Post(w.heldNote(ctx, d))
			}
		}
	case w.held[id]:
		delete(w.held, id)
		w.n.Clear(HeldID(id))
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
	for _, m := range []map[string]bool{w.held, w.msgs} {
		for id := range m {
			if open[id] {
				continue
			}
			if d, ok := w.s.Inbox.Get(id); ok {
				w.delivery(ctx, d, false, post)
				continue
			}
			if w.held[id] {
				delete(w.held, id)
				w.n.Clear(HeldID(id))
			}
			if w.msgs[id] {
				delete(w.msgs, id)
				w.n.Clear(MessageID(id))
			}
		}
	}
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

func (w *watcher) heldNote(ctx context.Context, d inbox.Delivery) Note {
	id, user := d.ID, w.s.User
	to := "you"
	switch {
	case d.To == "" && d.RepoKey != "":
		to = "repo " + d.RepoKey
	case d.To != user:
		to = w.name(d.To)
	}
	from := d.Message.From.Name
	if from == "" {
		from = w.name(d.Message.From.Key)
	}
	act := func(what string, fn func(context.Context, string, string) error) func() {
		return func() {
			if err := fn(ctx, id, user); err != nil {
				log.Printf("irori: notifications: %s %s: %v", what, id, err)
				w.n.Post(Note{ID: HeldID(id), Title: "Couldn't " + what + " that message", Body: err.Error(), Level: Error})
			}
		}
	}
	open := Target{Tab: TabInbox, Ref: id}
	return Note{
		ID:     HeldID(id),
		Title:  fmt.Sprintf("%s sent %s a message Irori held", from, to),
		Body:   upper(HoldReason(d.Hold)) + ": " + preview(d.Message.Text),
		Level:  Warn,
		Sticky: true,
		Actions: []Action{
			{Label: "Release", Run: act("release", w.s.Inbox.Release)},
			{Label: "Dismiss", Run: act("dismiss", w.s.Inbox.Dismiss)},
		},
		OnClick: func() { w.n.Open(open) },
		OnDismiss: func() {
			select {
			case w.forget <- id:
			default: // full: it stays held, and its card is back after a restart
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
}

// agentMoved says whether t's move into Needs you is news to the person:
// not their own doing (they claimed it, assigned it to themselves, blocked
// it on themselves), and about someone besides them.
func (w *watcher) agentMoved(t coord.Task) bool {
	if t.UpdatedBy == w.s.User {
		return false
	}
	return w.needsKey(t) != ""
}

// needsKey is who a Needs you card is about: its first holder that isn't
// the person, else its creator unless that is the person; "" for neither.
func (w *watcher) needsKey(t coord.Task) string {
	for _, h := range t.Holders {
		if h != w.s.User {
			return h
		}
	}
	if t.CreatedBy != w.s.User {
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
	case slices.Contains(t.Holders, w.s.User):
		title, body = agent+" handed you a card", t.Title
	default:
		title, body = agent+" needs you", t.Title
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

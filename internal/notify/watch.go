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
//     acting through the inbox as the person; cleared when it leaves Held;
//   - a Board card that moved into the person's Needs you lane (blocked on
//     them, waiting on their input, or assigned to them): Answer opens the
//     Board at the card, Later closes it; cleared when it leaves the lane;
//   - a message addressed to the person (User): Open shows it in the Inbox;
//     cleared once it is no longer waiting.
//
// What already needed the person when Watch starts doesn't pop up: the
// Inbox and Board counts and the tray carry it. A card body click opens the
// window at the card's tab (Notifier.Open).
func Watch(ctx context.Context, n *Notifier, s Sources) {
	if n.Headless() {
		return
	}
	w := &watcher{n: n, s: s, held: map[string]bool{}, msgs: map[string]bool{}, needs: map[string]bool{}}
	var ich <-chan inbox.Change
	var cch <-chan coord.Change
	if s.Inbox != nil {
		ich = s.Inbox.Subscribe(ctx)
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
		case _, ok := <-cch:
			if !ok {
				cch = nil
				continue
			}
			w.board(true)
		}
	}
}

type watcher struct {
	n     *Notifier
	s     Sources
	held  map[string]bool // delivery ids with a held card posted
	msgs  map[string]bool // delivery ids to the person with a card posted
	needs map[string]bool // task ids in Needs you (card posted, or seeded)
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
	id := d.ID
	// Held deliveries.
	switch {
	case d.State == inbox.Held && d.Hold != inbox.HoldPaused:
		if c.Kind == inbox.HeldNew || !w.held[id] {
			w.held[id] = w.n.Post(w.heldNote(ctx, d))
		}
	case w.held[id]:
		delete(w.held, id)
		w.n.Clear(HeldID(id))
	}
	// Messages to the person.
	if w.s.User == "" || d.To != w.s.User {
		return
	}
	waiting := d.State == inbox.Queued || d.State == inbox.Delivered
	switch {
	case waiting && !w.msgs[id]:
		w.msgs[id] = true
		w.n.Post(w.messageNote(d))
	case !waiting && w.msgs[id]:
		delete(w.msgs, id)
		w.n.Clear(MessageID(id))
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
		if post {
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

// needsAgent is the agent a Needs you card is about: its first agent
// holder, else its creator.
func (w *watcher) needsAgent(t coord.Task) string {
	for _, h := range t.Holders {
		if !strings.HasPrefix(h, "user:") {
			return w.name(h)
		}
	}
	if strings.HasPrefix(t.CreatedBy, "user:") {
		return "You"
	}
	return w.name(t.CreatedBy)
}

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

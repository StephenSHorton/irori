package tools

import (
	"context"
	"errors"
	"strings"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
)

// A message about a card (its Ref is task:<id>, or comment:<id> on a card)
// is also an entry on that card's thread: the person and any agent reading
// the card see it. The tools record it after the inbox call returns, never
// under the inbox's lock; the recipient-gone fallback records it again,
// marked gone (RecordGone). Its text goes on the card only once the wake
// policy has let it through (HeldOnCard): until then the entry is held,
// and RecordPassed brings the text.

// ErrNoCard: the message's Ref names no card here.
var ErrNoCard = errors.New("no card")

// Record says how RecordOnCard writes the entry.
type Record struct {
	Gone, Held bool   // the recipient ended; the first delivery wasn't let through (no text)
	Fill       bool   // the recipient's device let it through: the text for a held entry
	Writer     string // "irori" writes the row as Irori itself, else as the sender
}

// HeldOnCard reports whether d's message goes on its card without its
// text: d is held, or another device hosts its recipient and decides the
// hold with its own policy (it fills the text in once it lets d through).
func HeldOnCard(ib *inbox.Inbox, d inbox.Delivery) bool {
	return d.State == inbox.Held || !ib.Owned(d.ID)
}

// cardFor is the task a message Ref names: task:<id>, or comment:<id> on a
// card. ok is false for any other ref, or a comment not known here.
func cardFor(co *coord.Coord, ref string) (string, bool) {
	kind, id, _ := strings.Cut(ref, ":")
	switch kind {
	case "task":
		id = taskID(ref)
	case "comment":
		cm, found := co.GetComment(strings.TrimSpace(id))
		if !found {
			return "", false
		}
		id = cm.TaskID
	default:
		return "", false
	}
	return id, id != ""
}

// RecordOnCard puts m (to is its recipient's key) on the card its Ref
// names, attachments as a count. It is idempotent: the entry's id is the
// message id. ErrNoCard when the Ref names no card; coord.ErrNotFound when
// that card isn't here.
func RecordOnCard(ctx context.Context, co *coord.Coord, m inbox.Message, to string, r Record) (string, error) {
	tid, ok := cardFor(co, m.Ref)
	if !ok {
		return "", ErrNoCard
	}
	cm, err := co.RecordMessage(ctx, coord.MessageEntry{
		TaskID: tid, MsgID: m.ID, From: m.From.Key, To: to, Text: m.Text,
		Files: len(m.Attachments), Held: r.Held, Gone: r.Gone, Fill: r.Fill, Writer: r.Writer,
	})
	if err != nil {
		return "", err
	}
	return cm.TaskID, nil
}

// RecordGone is inbox.Options.OnRecipientGone over co: the message goes on
// its card marked gone, written by Irori itself. A ref that names no card
// here is inbox.ErrNoCard, so the message goes to the person instead.
func RecordGone(ctx context.Context, co *coord.Coord, m inbox.Message, to string) (string, error) {
	tid, err := RecordOnCard(ctx, co, m, to, Record{Gone: true, Writer: "irori"})
	if errors.Is(err, coord.ErrNotFound) || errors.Is(err, ErrNoCard) {
		return "", inbox.ErrNoCard
	}
	return tid, err
}

// RecordPassed is inbox.Options.OnPassed over co: this device let m's
// delivery to to through its wake policy, so the text of an entry recorded
// held goes on its card, written by Irori itself.
func RecordPassed(ctx context.Context, co *coord.Coord, m inbox.Message, to string) error {
	_, err := RecordOnCard(ctx, co, m, to, Record{Fill: true, Writer: "irori"})
	return err
}

// recorded is the line a send or reply result ends with once m (to is its
// recipient) is on the card its Ref names; "" when it names none.
func (s *set) recorded(ctx context.Context, m inbox.Message, to string, held bool) string {
	co := s.b.Coord()
	if co == nil {
		return ""
	}
	tid, ok := cardFor(co, m.Ref)
	if !ok {
		if strings.HasPrefix(m.Ref, "comment:") {
			return " (sent, but not recorded on " + oneLine(m.Ref, 80) + ": no card has that comment here)"
		}
		return ""
	}
	if _, err := RecordOnCard(ctx, co, m, to, Record{Held: held}); err != nil {
		return " (sent, but not recorded on task:" + oneLine(tid, 80) + ": " + oneLine(err.Error(), 200) + ")"
	}
	if held {
		return " It is on task:" + oneLine(tid, 80) + "'s thread; its text joins it once the wake policy lets it through."
	}
	return " It is on task:" + oneLine(tid, 80) + "'s thread."
}

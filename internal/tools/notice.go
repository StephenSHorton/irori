package tools

import (
	"context"
	"errors"
	"fmt"
	"strings"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
)

// NoticeResult is what became of one recipient of a coord.Notice.
type NoticeResult struct {
	To       string
	Delivery inbox.Delivery // set when Err is nil
	Err      error
}

// RouteNotice sends a coord.Notice to each of its recipients through the
// inbox, as coord.Notice's contract says: from is the acting sender (the
// agent or person who commented, assigned or unblocked) or
// inbox.SystemSender for coord.SweepNotices; the mode is next_turn unless
// the notice asked to wake; the ref and the notice flag ride along, and the
// notice's kind is the message's reason (so the window can tell the
// assignment from a later notice on the same card). Core
// (its sweep and the GUI) and the board tools all route through here.
//
// The coord row is already written when this runs, so a recipient that
// can't be told (rate limited, gone from every roster) is reported in the
// results, never undone.
func RouteNotice(ctx context.Context, ib *inbox.Inbox, from inbox.Sender, n coord.Notice) []NoticeResult {
	if ib == nil || len(n.To) == 0 {
		return nil
	}
	mode := inbox.ModeNextTurn
	if n.Wake {
		mode = inbox.ModeWake
	}
	out := make([]NoticeResult, 0, len(n.To))
	for _, to := range n.To {
		if to == "" || to == from.Key {
			continue
		}
		text, earlier := n.Text, ""
		if n.Kind == coord.NoticeWatched && mode == inbox.ModeNextTurn {
			text, earlier = mergeWatched(ib, from, to, n)
		}
		_, d, err := ib.SendWith(ctx, from, inbox.Address{Kind: inbox.ToAgent, Value: to}, text,
			inbox.SendOptions{Mode: mode, Ref: n.Ref, Notice: true, Reason: string(n.Kind), Attachments: n.Attachments})
		if err == nil && earlier != "" {
			// The new digest carries the earlier one's changes.
			err = ib.Skip(ctx, to, earlier, "merged into a newer update on the card")
		}
		out = append(out, NoticeResult{To: to, Delivery: d, Err: err})
	}
	return out
}

// mergeWatched folds a watch digest still waiting, unread, for to's next
// turn (same card, same sender) into n's text, so a watcher hears one
// update per card at its turn, not one per sweep. It returns the text to
// send and the id of the waiting delivery it replaces ("" for none).
func mergeWatched(ib *inbox.Inbox, from inbox.Sender, to string, n coord.Notice) (string, string) {
	for _, d := range ib.For(to, false, 0) {
		m := d.Message
		if d.State != inbox.Queued || d.Mode != inbox.ModeNextTurn || !m.Notice || m.Ref != n.Ref || m.From.Key != from.Key {
			continue
		}
		if text, ok := coord.MergeWatch(m.Text, n.Text); ok {
			return text, d.ID
		}
	}
	return n.Text, ""
}

// NoticeErr joins the failures in rs (nil when every recipient was told).
func NoticeErr(rs []NoticeResult) error {
	var errs []error
	for _, r := range rs {
		if r.Err != nil {
			errs = append(errs, fmt.Errorf("notice to %s: %w", r.To, r.Err))
		}
	}
	return errors.Join(errs...)
}

// told says, for a tool result, who heard about it and how.
func (s *set) told(rs []NoticeResult, n names) string {
	if len(rs) == 0 {
		return ""
	}
	var next, woke, held, failed []string
	for _, r := range rs {
		who := n.who(r.To)
		switch {
		case r.Err != nil:
			failed = append(failed, who+" ("+oneLine(r.Err.Error(), 160)+")")
		case r.Delivery.State == inbox.Held:
			held = append(held, who)
		case r.Delivery.Mode == inbox.ModeWake:
			woke = append(woke, who)
		default:
			next = append(next, who)
		}
	}
	var out string
	add := func(list []string, what string) {
		if len(list) > 0 {
			out += " " + what + " " + strings.Join(list, ", ") + "."
		}
	}
	add(next, "Told at their next turn:")
	add(woke, "Woke:")
	add(held, "Held by the wake policy until the person releases it:")
	add(failed, "Couldn't tell:")
	return out
}

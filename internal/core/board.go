package core

import (
	"context"
	"errors"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/tools"
)

// The Core holds the private log and every board's (v0.5 §2.1); a tools
// Backend that loses either interface would silently act on the private
// log alone.
var _ tools.TaskRouter = (*Core)(nil)

// RouteNotices delivers coord Notices through the one notice router the
// tools and the sweep use (tools.RouteNotice: next_turn unless the notice
// asked to wake, with its Ref, marked as a notice). from is who caused it:
// the person (UserSender) from the GUI. Every recipient is tried; failures
// come back joined, since the coord row is already written.
func (c *Core) RouteNotices(ctx context.Context, from inbox.Sender, ns ...coord.Notice) error {
	var errs []error
	for _, n := range ns {
		errs = append(errs, tools.NoticeErr(c.RouteNotice(ctx, from, n)))
	}
	return errors.Join(errs...)
}

// recordGone is inbox.Options.OnRecipientGone: a message whose recipient
// session ended goes on the card its Ref names (task:<id>, or comment:<id>
// on a card), marked gone and written by Irori itself (tools.RecordGone). A
// ref that names no card here is inbox.ErrNoCard, so the message goes to
// the person instead.
func (c *Core) recordGone(ctx context.Context, m inbox.Message, to string) (string, error) {
	return tools.RecordGone(ctx, c.coord, m, to)
}

// recordPassed is inbox.Options.OnPassed: this device let a message about
// a card through its wake policy, so its text goes on the card's entry,
// recorded held until now (tools.RecordPassed).
func (c *Core) recordPassed(ctx context.Context, m inbox.Message, to string) error {
	return tools.RecordPassed(ctx, c.coord, m, to)
}

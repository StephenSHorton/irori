package core

import (
	"context"
	"errors"
	"fmt"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
)

// RouteNotices delivers coord Notices through the inbox, once per recipient
// (board.md "Contracts", Integration): next_turn unless the notice asked to
// wake, with its Ref, marked as a notice so it needs no reply. from is who
// caused it: the person (UserSender), the agent that commented or assigned,
// or inbox.SystemSender for SweepNotices.
//
// The coord row is already written when a notice exists, so one recipient
// that can't be reached (a session that has gone) doesn't stop the others:
// every recipient is tried and the failures come back joined.
func (c *Core) RouteNotices(ctx context.Context, from inbox.Sender, ns ...coord.Notice) error {
	var errs []error
	for _, n := range ns {
		mode := inbox.ModeNextTurn
		if n.Wake {
			mode = inbox.ModeWake
		}
		for _, to := range n.To {
			_, _, err := c.inbox.SendWith(ctx, from, inbox.Address{Kind: inbox.ToAgent, Value: to}, n.Text,
				inbox.SendOptions{Mode: mode, Ref: n.Ref, Notice: true})
			if err != nil {
				errs = append(errs, fmt.Errorf("tell %s: %w", to, err))
			}
		}
	}
	return errors.Join(errs...)
}

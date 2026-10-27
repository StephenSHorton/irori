package core

import (
	"context"
	"errors"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/tools"
)

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

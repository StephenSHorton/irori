package core

import (
	"context"
	"errors"
	"log"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/tools"
)

// RouteNotice tells a coord.Notice's recipients through the inbox
// (tools.RouteNotice: next_turn unless the notice asked to wake). from is
// whoever acted: UserSender for the GUI, the agent for a tool call. The GUI's
// Board calls it after Assign, AddComment and Unblock.
func (c *Core) RouteNotice(ctx context.Context, from inbox.Sender, n coord.Notice) []tools.NoticeResult {
	return tools.RouteNotice(ctx, c.inbox, from, n)
}

// sweepCoord runs coord's sweeper and routes the notices for blockers that
// resolved by themselves, from Irori itself. A notice that can't be routed
// is logged; the blocker stays closed.
//
// The asks' timed part (coord.SweepAsks, board.md "Asks") runs with it,
// against the mesh roster: an ask's notices go from the person.
func (c *Core) sweepCoord(ctx context.Context) error {
	ns, err := c.coord.SweepNotices(ctx)
	as, aerr := c.coord.SweepAsks(ctx, c.MeshAgents())
	for _, n := range append(ns, as...) {
		from := inbox.SystemSender()
		if n.Person {
			from = c.UserSender()
		}
		if rerr := tools.NoticeErr(c.RouteNotice(ctx, from, n)); rerr != nil && ctx.Err() == nil {
			log.Printf("coord notice %s %s: %v", n.Kind, n.Ref, rerr)
		}
	}
	return errors.Join(err, aerr)
}

// staleNotice is inbox.Options.Stale: a notice about an ask someone else
// took (or that closed) is closed as skipped before it reaches an agent.
// coord.StaleNotice takes only a lock of its own, so the inbox may call it
// under its lock.
func (c *Core) staleNotice(d inbox.Delivery) string {
	if c.coord == nil {
		return ""
	}
	return c.coord.StaleNotice(d.Message.Ref, d.To)
}

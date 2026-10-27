package inbox

import (
	"context"
	"slices"
	"time"

	"github.com/StephenSHorton/irori/internal/presence"
)

// sweepItem is what planSweep sees of one open delivery.
type sweepItem struct {
	ID          string
	To          string
	From        string // the message's sender: repo work never goes back to it
	Repo        string // "" = agent-addressed work
	GoneRepo    string // owner-routed PR work (Repo ""): the PR's repo, where it goes once its owner has gone for good
	State       State
	Hold        HoldReason
	AssignedAt  time.Time // To last changed
	UpdatedAt   time.Time // last transition
	Until       time.Time // deferral due
	InFlight    bool      // Queued and handed to a wake not yet confirmed delivered
	IssuedAt    time.Time // that wake's issue time
	MovedFrom   bool      // created by a move: unengaged work never moves twice
	EngagedOnce bool      // its recipient engaged or deferred it
	Reminded    bool
}

// sweepEnv is the world planSweep plans against.
type sweepEnv struct {
	Now                           time.Time
	Unengaged, Gone, Quiet, Grace time.Duration
	Paused                        bool
	GoneFor                       func(agent string) time.Duration // 0 while around
	Reachable                     func(agent string) bool
	Desk                          func(repo string, exclude ...string) string // next front-desk candidate not in exclude, "" if none
}

type actionKind int

const (
	actReclaim actionKind = iota // in-flight wake never confirmed: back to queued
	actRequeue                   // deferral due
	actMove                      // repo work to another desk (To), or unowned when To == ""
	actAssign                    // unowned repo work gets its first desk
	actRemind                    // one reminder wake
)

type sweepAction struct {
	Kind       actionKind
	DeliveryID string
	To         string
}

// planSweep is the sweeper's pure planner (design §4; tsukumo planSweep and
// planReminders). At most one action per delivery.
func planSweep(items []sweepItem, env sweepEnv) []sweepAction {
	var out []sweepAction
	for _, it := range items {
		if a, ok := planOne(it, env); ok {
			out = append(out, a)
		}
	}
	return out
}

func planOne(it sweepItem, env sweepEnv) (sweepAction, bool) {
	act := func(k actionKind, to string) (sweepAction, bool) {
		return sweepAction{Kind: k, DeliveryID: it.ID, To: to}, true
	}
	switch it.State {
	case Held:
		// Only unowned repo work moves on its own; a person decides the rest.
		if it.Hold == HoldUnowned && it.Repo != "" {
			if to := env.Desk(it.Repo, it.From); to != "" {
				return act(actAssign, to)
			}
		}
		return sweepAction{}, false
	case Queued, Delivered, Engaged, Deferred:
	default:
		return sweepAction{}, false
	}

	// Repo work (and PR work, owner-routed too) whose holder has really
	// gone moves, whatever its state; agent work stays for the agent (it may
	// come back or be revived).
	gr := it.Repo
	if gr == "" {
		gr = it.GoneRepo
	}
	if gr != "" && it.To != "" && env.GoneFor(it.To) > env.Gone {
		return act(actMove, env.Desk(gr, it.To, it.From))
	}
	if it.InFlight && env.Now.Sub(it.IssuedAt) > env.Grace {
		return act(actReclaim, "")
	}
	if it.State == Deferred {
		if !env.Now.Before(it.Until) {
			return act(actRequeue, "")
		}
		return sweepAction{}, false
	}
	// Repo work the holder hasn't started on moves to the next desk, once;
	// after that (or with nobody to take it) the holder is reminded, once.
	if it.Repo != "" && !it.EngagedOnce && (it.State == Queued || it.State == Delivered) &&
		env.Now.Sub(it.AssignedAt) > env.Unengaged {
		if !it.MovedFrom {
			if to := env.Desk(it.Repo, it.To, it.From); to != "" {
				return act(actMove, to)
			}
		}
		if it.State == Delivered && !it.Reminded && !env.Paused && env.Reachable(it.To) {
			return act(actRemind, it.To)
		}
		return sweepAction{}, false
	}
	// One reminder for work left open: engaged with no reply and no deferral
	// for an hour, or delivered and never engaged (the host may have dropped
	// the prompt) past the unengaged limit, or an hour once it was engaged.
	quiet := env.Quiet
	if it.State == Delivered && !it.EngagedOnce {
		quiet = env.Unengaged
	}
	if (it.State == Engaged || it.State == Delivered) && !it.Reminded && !env.Paused &&
		env.Now.Sub(it.UpdatedAt) > quiet && env.Reachable(it.To) {
		return act(actRemind, it.To)
	}
	return sweepAction{}, false
}

func (b *inbox) sweep(ctx context.Context) error {
	b.mu.Lock()
	defer b.unlock()
	now := b.now()
	cards, err := b.liveCards(ctx)
	if err != nil {
		return err
	}

	// Track how long each recipient has been gone: not live in presence, or
	// no wake path (past the reattach grace).
	gone := func(agent string) bool {
		_, live := cards[agent]
		return !live || !b.reachable(agent, now)
	}
	seen := map[string]bool{}
	for _, d := range b.open {
		if d.To == "" || isUserKey(d.To) || seen[d.To] {
			continue
		}
		seen[d.To] = true
		if gone(d.To) {
			if _, ok := b.goneSince[d.To]; !ok {
				b.goneSince[d.To] = now
			}
		} else {
			delete(b.goneSince, d.To)
		}
	}
	for a := range b.goneSince {
		if !seen[a] {
			delete(b.goneSince, a)
		}
	}

	env := sweepEnv{
		Now: now, Unengaged: b.lim.unengaged, Gone: b.lim.gone, Quiet: b.lim.quiet, Grace: b.lim.reattach,
		Paused: b.pol.Paused,
		GoneFor: func(a string) time.Duration {
			if t, ok := b.goneSince[a]; ok {
				return now.Sub(t)
			}
			return 0
		},
		Reachable: func(a string) bool { return b.reachable(a, now) },
		Desk:      func(repo string, exclude ...string) string { return b.desk(repo, cards, now, exclude...) },
	}
	items := make([]sweepItem, 0, len(b.open))
	for _, id := range b.order {
		d := b.open[id]
		if d == nil {
			continue
		}
		it := sweepItem{
			ID: d.ID, To: d.To, Repo: d.RepoKey, State: d.State, Hold: d.Hold,
			AssignedAt: d.assignedAt, UpdatedAt: d.UpdatedAt, Until: d.Until,
			MovedFrom: d.movedFrom != "", EngagedOnce: d.engagedOnce, Reminded: d.Reminded,
		}
		m := b.msgs[d.MessageID]
		if m != nil {
			it.From = m.From.Key
		}
		it.GoneRepo = prFallbackRepo(m, d)
		if d.State == Queued && d.WakeID != "" {
			if w := b.wakes[d.WakeID]; w != nil && !w.Reminder {
				it.InFlight, it.IssuedAt = true, w.issuedAt
			}
		}
		items = append(items, it)
	}

	for _, a := range planSweep(items, env) {
		d := b.dels[a.DeliveryID]
		r := d.row()
		switch a.Kind {
		case actReclaim:
			r.WakeID = ""
			if err := b.putDelivery(ctx, r, "irori"); err != nil {
				return err
			}
		case actRequeue:
			r.State, r.Until, r.WakeID = Queued, time.Time{}, ""
			if err := b.putDelivery(ctx, r, "irori"); err != nil {
				return err
			}
		case actAssign:
			r.To = a.To
			if m := b.msgs[d.MessageID]; m != nil {
				r = placed(r, b.hold(m, a.To, cards))
			}
			if err := b.putDelivery(ctx, r, "irori"); err != nil {
				return err
			}
		case actMove:
			if err := b.move(ctx, d, a.To, cards); err != nil {
				return err
			}
		case actRemind:
			if !slices.Contains(b.reminders[a.To], d.ID) {
				b.reminders[a.To] = append(b.reminders[a.To], d.ID)
				b.touch(a.To)
			}
		}
	}
	// Reconcile WakeReady for everyone: some gates open with time alone (a
	// delivered wake going stale), with no mutation to notice it.
	for agent := range b.agentsWithOpen() {
		b.touch(agent)
	}
	return nil
}

// move hands d's message to a new delivery for to ("" = unowned), then
// closes d as Moved. The new row goes first so a crash between them can
// duplicate the work but never drop it.
func (b *inbox) move(ctx context.Context, d *drec, to string, cards map[string]presence.AgentCard) error {
	m := b.msgs[d.MessageID]
	nr := deliveryRow{DeliveryID: "dlv_" + b.ids.next(b.now()), MessageID: d.MessageID, To: to, RepoKey: prFallbackRepo(m, d), MovedFrom: d.ID, For: d.For}
	if d.RepoKey == "" && nr.RepoKey != "" {
		nr.For = clip(b.describe(d.To, cards), MaxNote) // owner-routed PR work: its owner has gone
	}
	h := HoldUnowned
	if m != nil {
		h = b.hold(m, to, cards)
	}
	if err := b.putDelivery(ctx, placed(nr, h), "irori"); err != nil {
		return err
	}
	r := d.row()
	r.State, r.MovedTo, r.WakeID = Moved, to, ""
	return b.putDelivery(ctx, r, "irori")
}

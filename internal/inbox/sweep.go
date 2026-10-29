package inbox

import (
	"context"
	"errors"
	"log"
	"slices"
	"strings"
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
	NextTurn    bool // ModeNextTurn: never moved for being unengaged, never reminded
	// The message: what it is about, and who sent it. Agent work whose
	// recipient ended goes on the card its Ref names, or to the person;
	// notices, Irori's own and the person's own about no card stay.
	Ref        string
	Notice     bool
	System     bool // from SystemSender
	FromPerson bool
}

// sweepEnv is the world planSweep plans against.
type sweepEnv struct {
	Now                           time.Time
	Unengaged, Gone, Quiet, Grace time.Duration
	Paused                        bool
	GoneFor                       func(agent string) time.Duration // 0 while around
	EndedFor                      func(agent string) time.Duration // out of presence, however reachable; 0 while around
	Self                          string                           // this device: ended agent work with no card goes to "user:"+Self
	Reachable                     func(agent string) bool
	Desk                          func(repo string, exclude ...string) string // next front-desk candidate not in exclude, "" if none
}

type actionKind int

const (
	actReclaim    actionKind = iota // in-flight wake never confirmed: back to queued
	actRequeue                      // deferral due
	actMove                         // repo work to another desk (To), or unowned when To == ""
	actAssign                       // unowned repo work gets its first desk
	actRemind                       // one reminder wake
	actGoneCard                     // agent work its recipient ended on, about a card: on the card after the sweep, then Skipped
	actGonePerson                   // agent work its recipient ended on, from an agent, about no card: to the person (To)
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
	// gone moves, whatever its state; agent work stays for the agent while
	// its session is in presence (it may reattach), until it has ended.
	gr := it.Repo
	if gr == "" {
		gr = it.GoneRepo
	}
	if gr != "" && it.To != "" && env.GoneFor(it.To) > env.Gone {
		return act(actMove, env.Desk(gr, it.To, it.From))
	}
	// Agent work whose recipient session has ended (out of presence past the
	// grace; a live session with no wake path keeps its mail) never waits
	// for it: it goes on the card it is about, whoever sent it, or else to
	// the person when an agent sent it. The person's own message about no
	// card, a notice (its content is on the card) and Irori's own stay.
	if gr == "" && it.To != "" && !it.Notice && !it.System && env.EndedFor(it.To) > env.Gone {
		switch {
		case refNamesCard(it.Ref):
			return act(actGoneCard, "")
		case !it.FromPerson:
			return act(actGonePerson, "user:"+env.Self)
		}
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
	// A next_turn delivery waits for the recipient's next turn, however long
	// that is: the rest of the sweep (unengaged moves, reminders) is for wakes.
	if it.NextTurn {
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

// refNamesCard reports whether a message ref names a Board card: a task,
// or a comment on one.
func refNamesCard(ref string) bool {
	return strings.HasPrefix(ref, "task:") || strings.HasPrefix(ref, "comment:")
}

// handoff is agent work whose recipient ended, for the card its message
// names: the sweep collects it under b.mu, and handOff records it after.
type handoff struct {
	DeliveryID string
	Message    Message
	To         string
	FromPerson bool // the person's own message: with no card here, it stays queued
}

func (b *inbox) sweep(ctx context.Context) ([]handoff, error) {
	b.mu.Lock()
	defer b.unlock()
	now := b.now()
	cards, err := b.liveCards(ctx)
	if err != nil {
		return nil, err
	}

	// Track how long each recipient has been gone: not live in presence, or
	// no wake path (past the reattach grace).
	gone := func(agent string) bool {
		_, live := cards[agent]
		return !live || !b.reachable(agent, now)
	}
	seen := map[string]bool{}
	// Only this device's own recipients are tracked: another device's
	// agent isn't in presence here, and its host sweeps its work (§6.3).
	for _, d := range b.open {
		if d.To == "" || b.isOwner(d.To) || seen[d.To] || !b.local(d.To) {
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
		// Ended is presence alone: a session still running with no wake
		// path (no mod attached) hasn't ended, and keeps its mail.
		if _, live := cards[d.To]; !live {
			if _, ok := b.endedSince[d.To]; !ok {
				b.endedSince[d.To] = now
			}
		} else {
			delete(b.endedSince, d.To)
		}
	}
	for a := range b.goneSince {
		if !seen[a] {
			delete(b.goneSince, a)
		}
	}
	for a := range b.endedSince {
		if !seen[a] {
			delete(b.endedSince, a)
		}
	}
	for id := range b.noCardSince {
		if b.open[id] == nil {
			delete(b.noCardSince, id)
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
		EndedFor:  func(a string) time.Duration { return b.endedFor(a, now) },
		Self:      b.dev,
		Reachable: func(a string) bool { return b.reachable(a, now) },
		Desk:      func(repo string, exclude ...string) string { return b.desk(repo, cards, now, exclude...) },
	}
	// The sweep plans only deliveries this device owns (v0.4 §6.3): a
	// remote recipient is never moved as gone, and repo work with no owner
	// yet is assigned only by the device that wrote it. Another device's
	// work freezes while that device is offline.
	items := make([]sweepItem, 0, len(b.open))
	for _, id := range b.order {
		d := b.open[id]
		if d == nil || !b.owns(d) {
			continue
		}
		it := sweepItem{
			ID: d.ID, To: d.To, Repo: d.RepoKey, State: d.State, Hold: d.Hold,
			AssignedAt: d.assignedAt, UpdatedAt: d.UpdatedAt, Until: d.Until,
			MovedFrom: d.movedFrom != "", EngagedOnce: d.engagedOnce, Reminded: d.Reminded,
			NextTurn: d.nextTurn(),
		}
		m := b.msgs[d.MessageID]
		if m != nil {
			it.From = m.From.Key
			it.Ref, it.Notice = m.Ref, m.Notice
			it.System, it.FromPerson = m.From.isSystem(), b.isOwner(m.From.Key)
		}
		it.GoneRepo = prFallbackRepo(m, d)
		if d.State == Queued && d.WakeID != "" {
			if w := b.wakes[d.WakeID]; w != nil && !w.Reminder {
				it.InFlight, it.IssuedAt = true, w.issuedAt
			}
		}
		items = append(items, it)
	}

	var hs []handoff
	for _, a := range planSweep(items, env) {
		d := b.dels[a.DeliveryID]
		r := d.row()
		switch a.Kind {
		case actReclaim:
			r.WakeID = ""
			if err := b.putDelivery(ctx, r, "irori"); err != nil {
				return nil, err
			}
		case actRequeue:
			r.State, r.Until, r.WakeID = Queued, time.Time{}, ""
			if err := b.putDelivery(ctx, r, "irori"); err != nil {
				return nil, err
			}
		case actAssign:
			r.To = a.To
			if m := b.msgs[d.MessageID]; m != nil {
				r = placed(r, b.hold(m, a.To, cards))
			}
			if err := b.putDelivery(ctx, r, "irori"); err != nil {
				return nil, err
			}
		case actMove:
			if err := b.move(ctx, d, a.To, cards); err != nil {
				return nil, err
			}
		case actGoneCard:
			// The card is written outside b.mu (handOff): coord has its
			// own lock, and the Board must never wait on the inbox.
			if m := b.msgs[d.MessageID]; m != nil {
				hs = append(hs, handoff{DeliveryID: d.ID, Message: *m, To: d.To, FromPerson: b.isOwner(m.From.Key)})
			}
		case actGonePerson:
			if err := b.moveGone(ctx, d, a.To, cards); err != nil {
				return nil, err
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
	return hs, nil
}

// endedFor is how long agent has been out of presence, as the sweeper
// tracks it (endedSince); 0 while it is around.
func (b *inbox) endedFor(agent string, now time.Time) time.Duration {
	if t, ok := b.endedSince[agent]; ok {
		return now.Sub(t)
	}
	return 0
}

// handOff puts the sweep's card handoffs on their cards, with b.mu not
// held, then closes each delivery. The card goes first, so a crash between
// the two only repeats an idempotent card write. No card here (ErrNoCard,
// or no hook) is noCard; any other error leaves the delivery open for the
// next sweep.
func (b *inbox) handOff(ctx context.Context, hs []handoff) error {
	for _, h := range hs {
		if b.onGone != nil {
			tid, err := b.onGone(ctx, h.Message, h.To)
			switch {
			case err == nil:
				if err := b.closeGone(ctx, h, tid); err != nil {
					return err
				}
				continue
			case !errors.Is(err, ErrNoCard):
				log.Printf("inbox: recipient gone: %s to its card: %v", h.DeliveryID, err)
				continue
			}
		}
		if err := b.noCard(ctx, h); err != nil {
			return err
		}
	}
	return nil
}

// stillGone is h's delivery if it is still the open, owned work of the
// same recipient, and that recipient still out of presence past the grace;
// else nil (it was answered, moved, or its session came back meanwhile).
// The caller holds b.mu.
func (b *inbox) stillGone(h handoff) *drec {
	d := b.open[h.DeliveryID]
	if d == nil || d.State == Held || !b.owns(d) || d.To != h.To || b.endedFor(d.To, b.now()) <= b.lim.gone {
		return nil
	}
	if _, live := b.rosterCards()[d.To]; live {
		return nil
	}
	return d
}

// closeGone closes a delivery whose message is now on task taskID's card:
// Skipped, noting where it went.
func (b *inbox) closeGone(ctx context.Context, h handoff, taskID string) error {
	b.mu.Lock()
	defer b.unlock()
	d := b.stillGone(h)
	if d == nil {
		return nil
	}
	delete(b.noCardSince, d.ID)
	r := d.row()
	r.State, r.Note, r.Until = Skipped, clip(NoteRecipientGone+"; on task:"+taskID, MaxNote), time.Time{}
	return b.putDelivery(ctx, r, "irori")
}

// passRec is a delivery let through whose message text waits to go on its
// card: the message, its recipient, and when it was let through.
type passRec struct {
	msg, to string
	since   time.Time
}

// passed queues d's message for its card (Options.OnPassed) once this
// device, its owner, let d through its wake policy: a message about a card
// was recorded there held, or before its recipient's device decided.
// Notices aren't recorded on cards. b.mu is held.
func (b *inbox) passed(d *drec) {
	if b.onPassed == nil || d == nil || d.To == "" || !b.owns(d) {
		return
	}
	m := b.msgs[d.MessageID]
	if m == nil || m.Notice || !refNamesCard(m.Ref) {
		return
	}
	if _, ok := b.passing[d.ID]; !ok {
		b.passing[d.ID] = passRec{msg: d.MessageID, to: d.To, since: b.now()}
	}
}

// fillCards puts the text of the messages passed queued on their cards,
// with b.mu not held. A failure (the card may not have arrived from
// another device yet) is tried again on later calls, for NoCardGrace.
func (b *inbox) fillCards(ctx context.Context) {
	if b.onPassed == nil {
		return
	}
	type job struct {
		id string
		p  passRec
		m  Message
	}
	var jobs []job
	b.mu.Lock()
	for id, p := range b.passing {
		if m := b.msgs[p.msg]; m != nil {
			jobs = append(jobs, job{id, p, *m})
		}
		delete(b.passing, id)
	}
	b.mu.Unlock()
	var retry []job
	for _, j := range jobs {
		err := b.onPassed(ctx, j.m, j.p.to)
		switch {
		case err == nil:
		case ctx.Err() == nil && b.now().Sub(j.p.since) <= b.lim.noCard:
			retry = append(retry, j)
		default:
			log.Printf("inbox: %s was let through, but its text isn't on its card: %v", j.id, err)
		}
	}
	if len(retry) == 0 {
		return
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	for _, j := range retry {
		if _, ok := b.passing[j.id]; !ok {
			b.passing[j.id] = j.p
		}
	}
}

// noCard is a handoff whose card isn't here, re-checked under b.mu. The
// card may only not have arrived from another device yet, so the delivery
// waits for later sweeps; an agent's message goes to the person (moveGone)
// once the card has been missing for NoCardGrace, or at once with no hook
// to put it on one. The person's own message stays queued, as with no
// card: sending it back to them is noise.
func (b *inbox) noCard(ctx context.Context, h handoff) error {
	b.mu.Lock()
	defer b.unlock()
	d := b.stillGone(h)
	if d == nil {
		delete(b.noCardSince, h.DeliveryID)
		return nil
	}
	if h.FromPerson {
		return nil
	}
	if b.onGone != nil {
		since, ok := b.noCardSince[d.ID]
		if !ok {
			b.noCardSince[d.ID] = b.now()
			return nil
		}
		if b.now().Sub(since) <= b.lim.noCard {
			return nil
		}
	}
	delete(b.noCardSince, d.ID)
	cards, err := b.liveCards(ctx)
	if err != nil {
		return err
	}
	return b.moveGone(ctx, d, "user:"+b.dev, cards)
}

// moveGone hands agent work whose recipient ended to to (the person): the
// new delivery says who it was for, and d closes Moved, noted
// NoteRecipientGone.
func (b *inbox) moveGone(ctx context.Context, d *drec, to string, cards map[string]presence.AgentCard) error {
	return b.moveWith(ctx, d, to, cards, clip(NoteRecipientGone+": "+b.describe(d.To, cards), MaxNote), NoteRecipientGone)
}

// move hands d's message to a new delivery for to ("" = unowned), then
// closes d as Moved. The new row goes first so a crash between them can
// duplicate the work but never drop it.
func (b *inbox) move(ctx context.Context, d *drec, to string, cards map[string]presence.AgentCard) error {
	forText := d.For
	if d.RepoKey == "" && prFallbackRepo(b.msgs[d.MessageID], d) != "" {
		forText = clip(b.describe(d.To, cards), MaxNote) // owner-routed PR work: its owner has gone
	}
	return b.moveWith(ctx, d, to, cards, forText, "")
}

// moveWith is move with the new delivery's For, and a note for d ("" keeps
// d's own).
func (b *inbox) moveWith(ctx context.Context, d *drec, to string, cards map[string]presence.AgentCard, forText, note string) error {
	m := b.msgs[d.MessageID]
	nr := deliveryRow{DeliveryID: "dlv_" + b.ids.next(b.now()), MessageID: d.MessageID, To: to, RepoKey: prFallbackRepo(m, d), Mode: d.Mode, MovedFrom: d.ID, For: forText}
	h := HoldUnowned
	switch {
	case to != "" && !b.local(to):
		h = "" // its host decides with its own policy (v0.4 §6.3)
	case m != nil:
		h = b.hold(m, to, cards)
	}
	if err := b.putDelivery(ctx, placed(nr, h), "irori"); err != nil {
		return err
	}
	r := d.row()
	r.State, r.MovedTo, r.WakeID = Moved, to, ""
	if note != "" {
		r.Note = note
	}
	return b.putDelivery(ctx, r, "irori")
}

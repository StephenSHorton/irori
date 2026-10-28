package inbox

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"time"

	"github.com/StephenSHorton/irori/internal/owner"
	"github.com/StephenSHorton/irori/internal/store"
)

// Delivery and wakes across devices (v0.4 §6.0, §6.3). The sender's device
// writes message.sent and the first delivery row; everything after that is
// written by the delivery's owner: the device whose id prefixes To, or,
// while To is empty, the device that wrote the first row. The owner decides
// holds with its own policy, plans wakes into its own sessions and runs the
// sweep over its own deliveries. Other devices ask, with delivery.decided.

// KindDecided is a request about a held delivery from any device's GUI:
// {deliveryId, choice: release|dismiss|move, to?, by, atDevice, at}, actor
// "user". The owner applies the first one it folds and writes the real
// delivery.state; later ones are already handled.
const KindDecided = "delivery.decided"

// Choices a delivery.decided row carries.
const (
	ChoiceRelease = "release"
	ChoiceDismiss = "dismiss"
	ChoiceMove    = "move"
)

type decidedRow struct {
	DeliveryID string    `json:"deliveryId"`
	Choice     string    `json:"choice"`
	To         string    `json:"to,omitempty"`
	By         string    `json:"by"`
	AtDevice   string    `json:"atDevice,omitempty"`
	At         time.Time `json:"at,omitzero"`
}

// local reports whether agent is hosted on this device: its key names this
// device, or no device at all.
func (b *inbox) local(agent string) bool {
	dev := owner.DeviceOf(agent)
	return dev == "" || dev == b.dev
}

// owns reports whether this device owns d (§6.0).
func (b *inbox) owns(d *drec) bool {
	if d.To != "" {
		return b.local(d.To)
	}
	return d.firstWriter == "" || d.firstWriter == b.dev
}

// Follow folds other devices' rows as the mesh inserts them, until ctx
// ends (§6.1), and does this device's part as their owner: holds by its own
// policy, wakes into its own sessions, and the first request on a held
// delivery. The Core runs it once.
func (b *Inbox) Follow(ctx context.Context) { b.impl.follow(ctx) }

// CatchUp folds what other devices wrote and runs this device's owner pass
// (its own hold policy on their queued deliveries) once, now. The Core runs
// it before the API serves, so an attach stream can't get a wake for a
// delivery this device hasn't decided the hold of yet.
func (b *Inbox) CatchUp(ctx context.Context) { b.impl.catchUp(ctx) }

func (b *inbox) follow(ctx context.Context) {
	sub := b.st.Subscribe(ctx)
	b.catchUp(ctx)
	for ev := range sub {
		if ev.Seq == 0 {
			b.catchUp(ctx) // the store's resync signal
			continue
		}
		if ev.DeviceID == b.dev || !isReplayKind(ev.Kind) {
			continue
		}
		b.applyRemote(ctx, ev)
	}
}

func isReplayKind(kind string) bool {
	for _, k := range replayKinds {
		if k == kind {
			return true
		}
	}
	return false
}

// catchUp folds every other device's inbox row past the last one folded,
// each device in seq order, then decides what this device owns and nobody
// decided yet.
func (b *inbox) catchUp(ctx context.Context) {
	vec, err := b.st.Vector(ctx)
	if err != nil {
		return
	}
	if b.cutSince(vec) {
		b.refold(ctx)
	}
	for dev, top := range vec {
		if dev == b.dev {
			continue
		}
		for {
			b.mu.Lock()
			after := b.last[dev]
			b.mu.Unlock()
			if after >= top {
				break
			}
			evs, err := b.st.List(ctx, store.Filter{DeviceID: dev, AfterSeq: after, Kinds: replayKinds, Limit: store.MaxListLimit})
			if err != nil || len(evs) == 0 {
				break
			}
			for _, ev := range evs {
				b.applyRemote(ctx, ev)
			}
			if len(evs) < store.MaxListLimit {
				break
			}
		}
	}
	b.mu.Lock()
	defer b.unlock()
	b.ownerPass(ctx)
}

// cutSince reports a device whose rows the log now holds fewer of than
// were folded: a revoke's cut moved them to refused_events (§5.5).
func (b *inbox) cutSince(vec store.Vector) bool {
	b.mu.Lock()
	defer b.mu.Unlock()
	for dev, seq := range b.last {
		if vec[dev] < seq {
			return true
		}
	}
	return false
}

// refold rebuilds everything folded from the log, from the log as it is
// now: rows that moved to refused_events stop counting (§5.5, §6.1). What
// is not from rows (reachability, gone clocks, reminders, subscribers)
// stays.
func (b *inbox) refold(ctx context.Context) {
	b.mu.Lock()
	defer b.unlock()
	b.msgs, b.dels, b.order, b.open = map[string]*Message{}, map[string]*drec{}, nil, map[string]*drec{}
	b.counted, b.wakes, b.tokens = map[string]bool{}, map[string]*wakeRec{}, map[string]string{}
	b.aliases, b.known = map[string][]string{}, map[string]string{}
	b.pol = Policy{Mode: Everyone, Block: []string{}, Allow: []string{}}
	b.running, b.pairs = map[string]turnRow{}, map[string][]time.Time{}
	b.visibleAt, b.heldFirst = map[string]time.Time{}, map[string]bool{}
	// handled stays: a request already applied isn't applied again.
	b.last, b.requests, b.ignored, b.ignoredBy = map[string]int64{}, nil, 0, nil
	if err := b.replayLocked(ctx); err != nil {
		log.Printf("inbox: re-fold after a revoke's cut: %v", err)
	}
	b.changes = append(b.changes, Change{Kind: Updated}) // after replayLocked cleared them
}

// applyRemote folds one row of another device and does the owner's part.
func (b *inbox) applyRemote(ctx context.Context, ev store.Event) {
	b.mu.Lock()
	defer b.unlock()
	if ev.Seq <= b.last[ev.DeviceID] {
		return
	}
	b.last[ev.DeviceID] = ev.Seq
	if err := b.apply(ev); err != nil {
		log.Printf("inbox: fold %s %s/%d: %v", ev.Kind, ev.DeviceID, ev.Seq, err)
		return
	}
	switch ev.Kind {
	case KindDelivery:
		var r struct {
			DeliveryID string `json:"deliveryId"`
		}
		if json.Unmarshal(ev.Payload, &r) == nil {
			if d := b.dels[r.DeliveryID]; d != nil {
				if err := b.hostHold(ctx, d); err != nil && ctx.Err() == nil {
					log.Printf("inbox: hold %s: %v", d.ID, err)
				}
			}
		}
	case KindDecided:
		var r decidedRow
		if json.Unmarshal(ev.Payload, &r) == nil && ev.Actor == "user" && b.isOwner(r.By) {
			if err := b.applyDecided(ctx, r); err != nil && ctx.Err() == nil {
				log.Printf("inbox: decide %s: %v", r.DeliveryID, err)
			}
		}
	}
}

// ownerPass decides every owned delivery another device queued and nobody
// here decided, and applies requests that waited for this device (it was
// offline when they arrived, or they were folded at replay). b.mu is held.
func (b *inbox) ownerPass(ctx context.Context) {
	for _, id := range b.order {
		d := b.open[id]
		if d == nil {
			continue
		}
		if err := b.hostHold(ctx, d); err != nil && ctx.Err() == nil {
			log.Printf("inbox: hold %s: %v", d.ID, err)
		}
	}
	for _, r := range b.requests {
		if err := b.applyDecided(ctx, r); err != nil && ctx.Err() == nil {
			log.Printf("inbox: decide %s: %v", r.DeliveryID, err)
		}
	}
	b.requests = nil
}

// hostHold is the host deciding a hold (§6.3): a delivery to one of this
// device's agents that another device queued is only a suggestion until
// this device runs hold() with its own policy (block and allow lists,
// bypass, hop limit, Pause all) and writes held, or lets it wake. b.mu is
// held.
func (b *inbox) hostHold(ctx context.Context, d *drec) error {
	if d.State != Queued || d.Hold != "" || d.WakeID != "" || d.To == "" || !b.local(d.To) ||
		d.lastWriter == "" || d.lastWriter == b.dev || d.hostChecked {
		return nil
	}
	d.hostChecked = true
	m := b.msgs[d.MessageID]
	if m == nil {
		return nil
	}
	cards, err := b.liveCards(ctx)
	if err != nil {
		return err
	}
	h := b.hold(m, d.To, cards)
	if h == "" {
		b.touch(d.To)
		return nil
	}
	return b.putDelivery(ctx, placed(d.row(), h), "irori")
}

// applyDecided is the owner applying a request (§6.3): the first one it
// folds for a delivery wins, and only the person's (actor "user", checked
// at fold) count. On other devices the request only waits here, for its
// owner. b.mu is held.
func (b *inbox) applyDecided(ctx context.Context, r decidedRow) error {
	d := b.dels[r.DeliveryID]
	if d == nil || !b.owns(d) {
		return nil
	}
	if b.handled[r.DeliveryID] {
		return nil
	}
	b.handled[r.DeliveryID] = true
	switch r.Choice {
	case ChoiceRelease:
		if d.State == Held {
			return b.releaseLocked(ctx, d, r.By)
		}
	case ChoiceDismiss:
		if d.State == Held {
			return b.putDelivery(ctx, dismissed(d), r.By)
		}
	case ChoiceMove:
		if d.State.Open() && r.To != "" && r.To != d.To {
			cards, err := b.liveCards(ctx)
			if err != nil {
				return err
			}
			return b.move(ctx, d, r.To, cards)
		}
	}
	return nil
}

func dismissed(d *drec) deliveryRow {
	r := d.row()
	r.State = Dismissed
	return r
}

// request writes a delivery.decided row for a delivery another device owns.
// b.mu is held.
func (b *inbox) request(ctx context.Context, d *drec, choice, to, by string) error {
	if !b.isOwner(by) {
		return fmt.Errorf("%w: only the person decides a held delivery", ErrBadState)
	}
	return b.emit(ctx, by, KindDecided, decidedRow{DeliveryID: d.ID, Choice: choice, To: to, By: by, AtDevice: b.dev, At: b.now()})
}

// Move hands an open delivery to another agent (by is the person). On
// another device's delivery it is a request its owner applies.
func (b *Inbox) Move(ctx context.Context, deliveryID, to, by string) error {
	x := b.impl
	x.mu.Lock()
	defer x.unlock()
	d := x.dels[deliveryID]
	if d == nil {
		return fmt.Errorf("%w: delivery %q", ErrNotFound, deliveryID)
	}
	if !d.State.Open() {
		return badState(d, "move")
	}
	if !x.owns(d) {
		return x.request(ctx, d, ChoiceMove, to, by)
	}
	cards, err := x.liveCards(ctx)
	if err != nil {
		return err
	}
	return x.move(ctx, d, to, cards)
}

// Owned reports whether this device owns the delivery (§6.0): it decides
// holds, wakes and sweeps for it, and a Release here takes effect at once.
func (b *Inbox) Owned(deliveryID string) bool {
	x := b.impl
	x.mu.Lock()
	defer x.mu.Unlock()
	d := x.dels[deliveryID]
	return d != nil && x.owns(d)
}

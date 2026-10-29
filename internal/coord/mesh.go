package coord

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/owner"
	"github.com/StephenSHorton/irori/internal/store"
)

// The Board across devices (v0.4 §6.4). task.*, lease.* and blocker.* rows
// replicate, so one Board shows every device's work. Leases are soft:
//   - a lease held by another device's agent never lapses by this device's
//     clock (its Until is the writer's wall clock); it ends only with a row
//     from its holder's device (a release, or the expiry its own Core
//     writes), a lease.stolen, or a revoke;
//   - two devices that acquired one resource while partitioned both hold
//     it, marked conflict; the lease of the lower device id stands, and the
//     other holder's Core writes lease.refused for its own lease;
//   - only the holder's device releases a lease automatically (its holder
//     gone, or its time up).

// KindLeaseStolen is the person ending a lease from any device:
// {resource, holder, acquiredAt, by, at}, actor "user". It ends exactly
// the lease taken at acquiredAt, so a renewal the holder wrote on the other
// side of a partition (it keeps acquiredAt) doesn't revive it.
const KindLeaseStolen = "lease.stolen"

// ReasonExpired is the release a holder's own Core writes when its lease's
// time is up, so other devices end it too.
const ReasonExpired = "expired"

// lapsedWithin bounds which leases found lapsed at replay still get their
// expiry row.
const lapsedWithin = 24 * time.Hour

type leaseStolenRow struct {
	Resource   string    `json:"resource"`
	Holder     string    `json:"holder"`
	AcquiredAt time.Time `json:"acquiredAt"`
	By         string    `json:"by"`
	At         time.Time `json:"at"`
}

func stolenKey(resource, holder string, acquiredAt time.Time) string {
	return resource + "\x00" + holder + "\x00" + acquiredAt.UTC().Format(time.RFC3339Nano)
}

// localKey reports whether key (an agent or "user:<device>") lives on this
// device, or names none.
func (c *coord) localKey(key string) bool {
	dev := owner.DeviceOf(key)
	return dev == "" || dev == c.st.DeviceID()
}

// live reports whether l still holds at now: by its Until on this device;
// another device's always, unless that device was removed (§6.4, §3).
func (c *coord) live(l *leaseRec, now time.Time) bool {
	if c.localKey(l.Holder) {
		return l.Until.After(now)
	}
	return c.revoked == nil || !c.revoked(owner.DeviceOf(l.Holder))
}

// Follow folds other devices' rows as the mesh inserts them, until ctx
// ends (§6.1), and does this device's part as a holder: refusing its own
// lease that lost a conflict, and writing the expiry of its own leases.
// The Core runs it once.
func (c *Coord) Follow(ctx context.Context) { c.impl.follow(ctx) }

var followKinds = []string{"task.", "lease.", "blocker.", "take."}

func followed(kind string) bool {
	for _, p := range followKinds {
		if strings.HasPrefix(kind, p) {
			return true
		}
	}
	return false
}

func (c *coord) follow(ctx context.Context) {
	sub := c.st.Subscribe(ctx)
	c.catchUp(ctx)
	t := time.NewTicker(time.Second)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case ev, ok := <-sub:
			if !ok {
				return
			}
			if ev.Seq == 0 {
				c.catchUp(ctx)
				continue
			}
			if ev.DeviceID == c.st.DeviceID() || !followed(ev.Kind) {
				continue
			}
			c.applyRemote(ctx, ev)
		case <-t.C:
			c.mu.Lock()
			c.holderDuties(ctx)
			c.unlock()
		}
	}
}

func (c *coord) catchUp(ctx context.Context) {
	vec, err := c.st.Vector(ctx)
	if err != nil {
		return
	}
	c.refoldIfCut(ctx, vec)
	for dev, top := range vec {
		if dev == c.st.DeviceID() {
			continue
		}
		for {
			c.mu.Lock()
			after := c.last[dev]
			c.mu.Unlock()
			if after >= top {
				break
			}
			evs, err := c.st.List(ctx, store.Filter{DeviceID: dev, AfterSeq: after, Kinds: followKinds, Limit: store.MaxListLimit})
			if err != nil || len(evs) == 0 {
				break
			}
			for _, ev := range evs {
				c.applyRemote(ctx, ev)
			}
			if len(evs) < store.MaxListLimit {
				break
			}
		}
	}
	c.mu.Lock()
	c.holderDuties(ctx)
	c.unlock()
}

// refoldIfCut rebuilds the Board's fold from the log when it now holds
// fewer of some device's rows than were folded: a revoke's cut moved them
// to refused_events (§5.5), and the tasks, comments and leases they wrote
// must stop counting (§6.1). What isn't from rows (gone clocks, watchers'
// queues, subscribers) stays.
func (c *coord) refoldIfCut(ctx context.Context, vec store.Vector) {
	c.mu.Lock()
	defer c.unlock()
	cut := false
	for dev, seq := range c.last {
		if vec[dev] < seq {
			cut = true
		}
	}
	if !cut {
		return
	}
	shorts := make(map[string]string, len(c.tasks))
	for id, t := range c.tasks {
		shorts[id] = t.Short
	}
	c.tasks, c.order, c.nextOrder = map[string]*Task{}, map[string]uint64{}, 0
	c.held, c.comments, c.byTask, c.refusals = map[string]*leaseRec{}, map[string]*Comment{}, map[string][]string{}, nil
	c.blockerSeq, c.stepSeq, c.activity = map[string]int{}, map[string]int{}, map[string][]Activity{}
	// shortTop stays: the cut card's numbers aren't handed out again.
	c.claims, c.claimed, c.shortGroups, c.shortIdx = nil, map[string]bool{}, map[int][]shortClaim{}, map[string]string{}
	c.last, c.conflicts, c.stolen, c.lapsed, c.ignored, c.ignoredBy = map[string]int64{}, map[string]*leaseRec{}, map[string]bool{}, nil, 0, nil
	c.asks.reset()
	c.links, c.pins = nil, map[string]pinRec{}
	if err := c.replay(ctx); err != nil {
		log.Printf("coord: re-fold after a revoke's cut: %v", err)
	}
	// A card whose clash partner was cut lost its letter in the replay,
	// which notes nothing: note it as moved, as a live renumber would be.
	for id, t := range c.tasks {
		if was := shorts[id]; was != "" && t.Short != was {
			c.shortMovedTo(t, was, t.Short)
		}
	}
	c.note(Change{Kind: Resync})
}

func (c *coord) applyRemote(ctx context.Context, ev store.Event) {
	c.mu.Lock()
	defer c.unlock()
	if ev.Seq <= c.last[ev.DeviceID] {
		return
	}
	c.last[ev.DeviceID] = ev.Seq
	c.apply(ev)
	c.holderDuties(ctx)
}

// holderDuties writes what only this device, as a lease's holder, writes:
// lease.refused for its own lease that lost a conflict, and the release of
// its own leases whose time ran out. c.mu is held.
func (c *coord) holderDuties(ctx context.Context) {
	now := c.stamp()
	for name, l := range c.conflicts {
		// One refusal per lease (its AcquiredAt): a later lease of the same
		// holder that loses another conflict gets its own. Marked only once
		// written, so a failed write (appends held) is tried again.
		key := stolenKey(name, l.Holder, l.AcquiredAt)
		if !c.localKey(l.Holder) || c.refused[key] {
			continue
		}
		winner := c.held[name]
		heldBy := ""
		if winner != nil {
			heldBy = winner.Holder
		}
		if err := c.write(ctx, l.Holder, KindLeaseRefused, Refusal{
			Resource: name, By: l.Holder, HeldBy: heldBy, Held: name, Until: l.Until, At: now, Conflict: true,
		}); err != nil {
			if ctx.Err() == nil {
				log.Printf("coord: refuse conflicting lease %s: %v", name, err)
			}
			continue
		}
		c.refused[key] = true
	}
	c.prune(now)
	for _, l := range c.lapsed {
		if cur := c.held[l.Resource]; cur != nil && cur.Holder == l.Holder {
			continue // renewed since; the new lease stands
		}
		if err := c.write(ctx, "irori", KindLeaseRel, leaseReleasedRow{
			Resource: l.Resource, Holder: l.Holder, Reason: ReasonExpired, By: "irori", At: now,
		}); err != nil && ctx.Err() == nil {
			log.Printf("coord: record expired lease %s: %v", l.Resource, err)
		}
	}
	c.lapsed = c.lapsed[:0]
	c.askDuties(ctx)
}

// conflictOf folds an acquire that meets a live lease on the same resource
// from another device (§6.4): both are kept, the lower device id's stands
// in held, the other is the conflict. It reports whether it handled the
// row. c.mu is held.
func (c *coord) conflictOf(rec *leaseRec, stolen []stolenLease) bool {
	e := c.held[rec.res.Name]
	if e == nil || e.Holder == rec.Holder || owner.DeviceOf(e.Holder) == owner.DeviceOf(rec.Holder) {
		return false
	}
	for _, v := range stolen {
		if v.Holder == e.Holder {
			return false // a steal, not a conflict
		}
	}
	if !c.live(e, c.stamp()) {
		return false
	}
	// Two leases conflict only if their times overlap, give or take the
	// clock bound (§5.6). A lease that ended before the held one began is
	// history arriving late (a partition healing): the held one stands, and
	// nobody is told it lost. One that began after the held one ended takes
	// its place as usual.
	switch {
	case !rec.Until.Add(clockSkew).After(e.AcquiredAt):
		return true
	case !e.Until.Add(clockSkew).After(rec.AcquiredAt):
		return false
	}
	winner, loser := e, rec
	if owner.DeviceOf(rec.Holder) < owner.DeviceOf(e.Holder) {
		winner, loser = rec, e
	}
	winner.Conflict, loser.Conflict = loser.Holder, winner.Holder
	c.held[rec.res.Name] = winner
	c.conflicts[rec.res.Name] = loser
	c.note(Change{Kind: LeaseChanged, Resource: rec.res.Name, Holder: winner.Holder, Reason: ReasonConflict})
	return true
}

// clockSkew is how far two devices' wall clocks may differ (§5.6: a row
// more than 2 minutes ahead isn't stored yet).
const clockSkew = 2 * time.Minute

// ReasonConflict marks a lease another device's lease contests (§6.4).
const ReasonConflict = "conflict"

// StealLease is the person ending a lease held on any device (§6.4): one
// lease.stolen row ends it everywhere. by must be the person.
func (c *Coord) StealLease(ctx context.Context, by, resource string) error {
	x := c.impl
	if !x.isOwner(by) {
		return fmt.Errorf("%w: only the person steals a lease", ErrInvalid)
	}
	name, err := NormalizeResource(resource)
	if err != nil {
		return err
	}
	x.mu.Lock()
	defer x.unlock()
	l := x.held[name]
	if l == nil || !x.live(l, x.stamp()) {
		return fmt.Errorf("%w: no lease on %s", ErrNotFound, name)
	}
	return x.steal(ctx, by, l)
}

// steal writes lease.stolen for l. c.mu is held.
func (c *coord) steal(ctx context.Context, by string, l *leaseRec) error {
	return c.write(ctx, by, KindLeaseStolen, leaseStolenRow{
		Resource: l.Resource, Holder: l.Holder, AcquiredAt: l.AcquiredAt, By: by, At: c.stamp(),
	})
}

// applyStolen folds lease.stolen: only the person's (actor "user") count.
// c.mu is held.
func (c *coord) applyStolen(ev store.Event) {
	var p leaseStolenRow
	if json.Unmarshal(ev.Payload, &p) != nil || p.Resource == "" || p.Holder == "" {
		return
	}
	if ev.Actor != "user" || !c.isOwner(p.By) {
		c.ignore(ev.DeviceID)
		return
	}
	name := p.Resource
	if n, err := NormalizeResource(p.Resource); err == nil {
		name = n
	}
	c.stolen[stolenKey(name, p.Holder, p.AcquiredAt)] = true
	match := func(l *leaseRec) bool {
		return l != nil && l.Holder == p.Holder && l.AcquiredAt.Equal(p.AcquiredAt)
	}
	if match(c.conflicts[name]) || match(c.held[name]) {
		c.endConflict(name, p.Holder)
		c.note(Change{Kind: LeaseChanged, Resource: name, Holder: p.Holder, Reason: ReasonStolen})
	}
}

// endConflict ends holder's lease on name, held or contested, and reports
// whether there was one. When the lease that stood ends, the contested one
// stands alone. c.mu is held.
func (c *coord) endConflict(name, holder string) bool {
	if l := c.conflicts[name]; l != nil && l.Holder == holder {
		delete(c.conflicts, name)
		if w := c.held[name]; w != nil {
			w.Conflict = ""
		}
		return true
	}
	if l := c.held[name]; l != nil && l.Holder == holder {
		delete(c.held, name)
		if o := c.conflicts[name]; o != nil {
			delete(c.conflicts, name)
			o.Conflict = ""
			c.held[name] = o
		}
		return true
	}
	return false
}

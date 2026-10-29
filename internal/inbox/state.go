package inbox

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// More errors callers can branch on (additions to the contract's set).
var (
	ErrSelf  = errors.New("inbox: that's you; address another agent")
	ErrEmpty = errors.New("inbox: give the message text")
	// ErrBadPR: a ToPR address that isn't a PR.
	ErrBadPR = errors.New("inbox: not a pull request address")
)

// MaxNote caps defer/skip notes and send reasons (runes).
const MaxNote = 500

// subBuffer is each subscriber's channel size.
const subBuffer = 64

// replayKinds are the rows New folds. agent.seen and agent.name are read
// only to learn the keys and names of agents that have since gone, so they
// stay addressable.
var replayKinds = []string{
	KindMessageSent, KindDelivery, KindWakeIssued, KindAlias, KindAgentName,
	KindTurnEngaged, KindTurnEnded, store.KindAgentSeen, KindDecided,
}

// localReplayKinds are the device-local rows New folds (local_events).
var localReplayKinds = []string{KindPolicySet}

// limits are Options' durations with defaults filled in.
type limits struct {
	unengaged, gone, quiet, reattach, noCard time.Duration
}

// drec is a delivery plus the bookkeeping the sweeper needs. Message is left
// empty here and filled in when a copy goes out.
type drec struct {
	Delivery
	movedFrom   string    // the delivery this one replaced (a move); moved work never moves on unengaged again
	assignedAt  time.Time // when To last changed
	engagedOnce bool      // the current recipient engaged or deferred it at least once
	firstWriter string    // the device that wrote its first row: its owner while To is empty (v0.4 §6.0)
	lastWriter  string    // the device that wrote its newest row
	hostChecked bool      // this device, its host, ran hold() on another device's queued row (memory only)
}

// deliveryRow is the KindDelivery payload: a full snapshot of the delivery's
// mutable fields, so the fold is a replace.
type deliveryRow struct {
	DeliveryID  string       `json:"deliveryId"`
	MessageID   string       `json:"messageId"`
	To          string       `json:"to"`
	RepoKey     string       `json:"repoKey,omitempty"`
	Mode        DeliveryMode `json:"mode,omitempty"` // "" = ModeWake
	State       State        `json:"state"`
	Hold        HoldReason   `json:"hold,omitempty"`
	Via         string       `json:"via,omitempty"`
	WakeID      string       `json:"wakeId,omitempty"`
	Note        string       `json:"note,omitempty"`
	For         string       `json:"for,omitempty"`
	Until       time.Time    `json:"until,omitzero"`
	MovedTo     string       `json:"movedTo,omitempty"`
	MovedFrom   string       `json:"movedFrom,omitempty"`
	Reminded    bool         `json:"reminded,omitempty"`
	EngagedOnce bool         `json:"engagedOnce,omitempty"`
	By          string       `json:"by,omitempty"`
	At          time.Time    `json:"at,omitzero"`
}

// row snapshots d as a deliveryRow, ready to change and append.
func (d *drec) row() deliveryRow {
	return deliveryRow{
		DeliveryID: d.ID, MessageID: d.MessageID, To: d.To, RepoKey: d.RepoKey, Mode: d.Mode,
		State: d.State, Hold: d.Hold, Via: d.Via, WakeID: d.WakeID, Note: d.Note, For: d.For,
		Until: d.Until, MovedTo: d.MovedTo, MovedFrom: d.movedFrom,
		Reminded: d.Reminded, EngagedOnce: d.engagedOnce,
	}
}

type wakeRow struct {
	WakeID      string       `json:"wakeId"`
	To          string       `json:"to"`
	Token       string       `json:"token"`
	Reminder    bool         `json:"reminder"`
	DeliveryIDs []string     `json:"deliveryIds"`
	Path        Path         `json:"path,omitempty"`
	Mode        DeliveryMode `json:"mode,omitempty"` // "next_turn" for an append or a next_turn-only push; "" = wake
	Overflow    []string     `json:"overflow,omitempty"`
	At          time.Time    `json:"at,omitzero"`
}

type wakeRec struct {
	Wake
	path     Path
	issuedAt time.Time
}

// isAppend: w is a next-turn append (mod path), which never gates wakes.
func (w *wakeRec) isAppend() bool { return strings.HasPrefix(w.Token, appendTokenPrefix) }

// gates: w holds back agent's next wake until it is delivered and engaged.
// Reminders, appends and next_turn-only channel pushes never do: none of
// them asked the agent to act now.
func (w *wakeRec) gates() bool { return !w.Reminder && !w.isAppend() && w.Mode != ModeNextTurn }

type aliasRow struct {
	Key   string    `json:"key"`
	Alias string    `json:"alias"`
	At    time.Time `json:"at,omitzero"`
}

type nameRow struct {
	Key  string    `json:"key"`
	Name string    `json:"name"`
	At   time.Time `json:"at,omitzero"`
}

type policyRow struct {
	Policy
	By string    `json:"by"`
	At time.Time `json:"at,omitzero"`
}

type turnRow struct {
	Agent  string    `json:"agent"`
	WakeID string    `json:"wakeId"`
	TurnID string    `json:"turnId"`
	At     time.Time `json:"at,omitzero"`
}

// reachRec is an agent's wake paths (memory only: transports re-announce
// themselves after a restart).
type reachRec struct {
	mod, channel bool
	modLost      time.Time // when the mod path dropped; it counts for ReattachGrace after
	firstAt      time.Time // start of the current reachable stretch; orders the front desk
}

type inbox struct {
	mu     sync.Mutex
	st     *store.Store
	roster Roster
	owners func(prKey string) []string // Options.PROwners; nil = no ledger
	// onGone is Options.OnRecipientGone; nil sends agents' card messages to
	// the person. onPassed is Options.OnPassed.
	onGone   func(ctx context.Context, m Message, to string) (string, error)
	onPassed func(ctx context.Context, m Message, to string) error
	dev      string
	ownerFn  func(string) bool // Options.IsOwner
	// ignored counts owner-only rows folded from the wrong device (v0.4 §6.0).
	ignored   int
	ignoredBy map[string]int // and per writing device
	// Across devices (mesh.go): the highest seq folded per device, the
	// deliveries whose first request was applied here, and requests folded
	// at replay that wait for the owner pass.
	last     map[string]int64
	handled  map[string]bool
	requests []decidedRow

	now func() time.Time
	lim limits
	ids idGen

	msgs    map[string]*Message
	dels    map[string]*drec
	order   []string         // delivery ids, oldest first
	open    map[string]*drec // deliveries whose State.Open()
	counted map[string]bool  // messages whose first delivery fed the rate limit
	wakes   map[string]*wakeRec
	tokens  map[string]string   // token -> wake id
	aliases map[string][]string // agent key -> aliases, oldest first
	known   map[string]string   // agent key -> lowercased card name ("" if none) for every agent ever seen
	pol     Policy
	running map[string]turnRow     // agent -> the wake whose turn is running
	pairs   map[string][]time.Time // unordered pair -> send times

	reach     map[string]*reachRec
	goneSince map[string]time.Time
	// endedSince: when each recipient left presence, reachability aside
	// (the ended handoff, sweep.go). noCardSince: when each handoff first
	// found no card here. passing: deliveries let through whose message
	// text waits to go on its card (fillCards). Memory only.
	endedSince  map[string]time.Time
	noCardSince map[string]time.Time
	passing     map[string]passRec
	reminders   map[string][]string // agent -> delivery ids a reminder wake is due for
	submit      map[string]bool     // last hasSubmit per agent, for WakeReady edges
	appendable  map[string]bool     // last hasAppend per agent, for AppendReady edges
	// Search visibility (search.go): when each message first had a delivery
	// outside Held/Dismissed, and whether one was held before that.
	visibleAt map[string]time.Time
	heldFirst map[string]bool
	subs      map[chan Change]struct{}

	// Per-mutation scratch, flushed by unlock.
	replaying bool
	changes   []Change
	touched   map[string]bool
}

func newInbox(ctx context.Context, st *store.Store, roster Roster, opts Options) (*Inbox, error) {
	if st == nil {
		return nil, errors.New("inbox: nil store")
	}
	if roster == nil {
		return nil, errors.New("inbox: nil roster")
	}
	now := opts.Now
	if now == nil {
		now = time.Now
	}
	pick := func(v, def time.Duration) time.Duration {
		if v > 0 {
			return v
		}
		return def
	}
	ownerFn := opts.IsOwner
	if ownerFn == nil {
		self := "user:" + opts.DeviceID
		ownerFn = func(k string) bool { return k == self }
	}
	b := &inbox{
		st: st, roster: roster, owners: opts.PROwners, onGone: opts.OnRecipientGone, onPassed: opts.OnPassed, dev: opts.DeviceID, now: now,
		ownerFn: ownerFn,
		lim: limits{
			unengaged: pick(opts.UnengagedLimit, UnengagedLimit),
			gone:      pick(opts.GoneGrace, GoneGrace),
			quiet:     pick(opts.QuietLimit, QuietLimit),
			reattach:  pick(opts.ReattachGrace, ReattachGrace),
			noCard:    pick(opts.NoCardGrace, NoCardGrace),
		},
		msgs: map[string]*Message{}, dels: map[string]*drec{}, open: map[string]*drec{},
		counted: map[string]bool{}, wakes: map[string]*wakeRec{}, tokens: map[string]string{},
		aliases: map[string][]string{}, known: map[string]string{},
		pol:     Policy{Mode: Everyone, Block: []string{}, Allow: []string{}},
		running: map[string]turnRow{}, pairs: map[string][]time.Time{},
		reach: map[string]*reachRec{}, goneSince: map[string]time.Time{}, endedSince: map[string]time.Time{},
		noCardSince: map[string]time.Time{}, passing: map[string]passRec{},
		reminders: map[string][]string{}, submit: map[string]bool{}, appendable: map[string]bool{},
		visibleAt: map[string]time.Time{}, heldFirst: map[string]bool{},
		subs: map[chan Change]struct{}{}, touched: map[string]bool{},
		last: map[string]int64{}, handled: map[string]bool{},
	}
	if err := b.replay(ctx); err != nil {
		return nil, err
	}
	return &Inbox{impl: b}, nil
}

// replay folds every inbox row, oldest first, paging past List's cap.
func (b *inbox) replay(ctx context.Context) error {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.replayLocked(ctx)
}

// replayLocked is replay with b.mu held.
func (b *inbox) replayLocked(ctx context.Context) error {
	b.replaying = true
	defer func() { b.replaying = false }()
	// Replicated rows, then this device's policy rows (local_events, v0.4
	// §5.1). Policy folds as "newest wins" and nothing else reads it while
	// replaying, so the two passes give the same state as one.
	for _, local := range []bool{false, true} {
		kinds := replayKinds
		if local {
			kinds = localReplayKinds
		}
		var after store.Cursor
		for {
			evs, err := b.st.List(ctx, store.Filter{Kinds: kinds, After: after, Limit: store.MaxListLimit, Local: local})
			if err != nil {
				return fmt.Errorf("inbox: replay: %w", err)
			}
			for _, ev := range evs {
				if err := b.apply(ev); err != nil {
					return fmt.Errorf("inbox: replay %s %s/%d: %w", ev.Kind, ev.DeviceID, ev.Seq, err)
				}
				if !local && ev.Seq > b.last[ev.DeviceID] {
					b.last[ev.DeviceID] = ev.Seq
				}
				after = ev.Cursor()
			}
			if len(evs) < store.MaxListLimit {
				break
			}
		}
	}
	// Baseline for WakeReady edges; replay itself notifies nobody.
	for agent := range b.agentsWithOpen() {
		b.submit[agent] = b.hasSubmit(agent)
		b.appendable[agent] = b.hasAppend(agent)
	}
	b.changes = nil
	clear(b.touched)
	return nil
}

// emit appends one row and folds it. Every mutation goes through here, so
// live state and replayed state come from the same code.
func (b *inbox) emit(ctx context.Context, by, kind string, payload any) error {
	ev, err := b.st.Append(ctx, b.actorFor(by), kind, payload)
	if err != nil {
		return fmt.Errorf("inbox: append %s: %w", kind, err)
	}
	return b.apply(ev)
}

// putDelivery stamps and appends a delivery snapshot. A held delivery let
// through queues its message's text for its card (passed).
func (b *inbox) putDelivery(ctx context.Context, r deliveryRow, by string) error {
	wasHeld := false
	if d := b.dels[r.DeliveryID]; d != nil {
		wasHeld = d.State == Held
	}
	r.By = by
	r.At = b.now()
	if err := b.emit(ctx, by, KindDelivery, r); err != nil {
		return err
	}
	if wasHeld && r.State == Queued {
		b.passed(b.dels[r.DeliveryID])
	}
	return nil
}

// emitLocal appends one device-local row (policy.set) and folds it.
func (b *inbox) emitLocal(ctx context.Context, by, kind string, payload any) error {
	ev, err := b.st.AppendLocal(ctx, b.actorFor(by), kind, payload)
	if err != nil {
		return fmt.Errorf("inbox: append %s: %w", kind, err)
	}
	return b.apply(ev)
}

// actorFor maps a caller to the store's actor forms. Only the owner's own
// key (isOwner) makes a "user" row; the loopback API's "api:<sender>" keeps
// its own actor, and a made-up "user:..." key is just an agent key.
func (b *inbox) actorFor(by string) string {
	switch {
	case by == "" || by == "irori":
		return "irori"
	case b.isOwner(by):
		return "user"
	case strings.HasPrefix(by, APIKeyPrefix):
		return by
	default:
		return "agent:" + by
	}
}

// APIKeyPrefix starts the by of a row the loopback API writes on an
// agent's behalf ("api:<sender key>"): never a "user" row (v0.4 §2).
const APIKeyPrefix = "api:"

// isOwner reports whether key is the person's ("user:<deviceId>" for a
// device certified to them; Options.IsOwner).
func (b *inbox) isOwner(key string) bool { return b.ownerFn(key) }

// stamp is a row's time: its own "at", else its HLC wall time.
func stamp(at time.Time, ev store.Event) time.Time {
	if !at.IsZero() {
		return at
	}
	return ev.HLC.Wall()
}

// apply folds one row into memory.
func (b *inbox) apply(ev store.Event) error {
	if !b.writerOK(ev) {
		b.ignore(ev.DeviceID)
		return nil
	}
	switch ev.Kind {
	case KindMessageSent:
		var m Message
		if err := json.Unmarshal(ev.Payload, &m); err != nil {
			return err
		}
		if m.ID == "" {
			return errors.New("message without id")
		}
		b.msgs[m.ID] = &m
		if m.From.Key != "" && !m.From.IsUser() && !m.From.isSystem() {
			if _, ok := b.known[m.From.Key]; !ok {
				b.known[m.From.Key] = ""
			}
		}

	case KindDelivery:
		var r deliveryRow
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return err
		}
		if r.DeliveryID == "" {
			return errors.New("delivery without id")
		}
		at := stamp(r.At, ev)
		d, ok := b.dels[r.DeliveryID]
		prevTo, prevState := "", State("")
		if !ok {
			d = &drec{Delivery: Delivery{ID: r.DeliveryID, CreatedAt: at}}
			d.firstWriter = ev.DeviceID
			b.dels[r.DeliveryID] = d
			b.order = append(b.order, r.DeliveryID)
			d.assignedAt = at
			if !b.counted[r.MessageID] {
				b.counted[r.MessageID] = true
				if m := b.msgs[r.MessageID]; m != nil && r.MovedFrom == "" && b.countsForRate(m.From.Key, r.To) {
					k := pairKey(m.From.Key, r.To)
					b.pairs[k] = append(b.pairs[k], m.SentAt)
				}
			}
		} else {
			prevTo, prevState = d.To, d.State
			if prevTo != r.To {
				d.assignedAt = at
			}
		}
		d.MessageID, d.To, d.RepoKey, d.Mode = r.MessageID, r.To, r.RepoKey, r.Mode
		if d.Mode == "" {
			d.Mode = ModeWake // rows from before next-turn delivery
		}
		d.State, d.Hold, d.Via, d.WakeID = r.State, r.Hold, r.Via, r.WakeID
		d.Note, d.For, d.Until, d.MovedTo, d.Reminded = r.Note, r.For, r.Until, r.MovedTo, r.Reminded
		d.movedFrom, d.engagedOnce = r.MovedFrom, r.EngagedOnce
		d.UpdatedAt = at
		d.lastWriter = ev.DeviceID
		if d.State.Open() {
			b.open[d.ID] = d
		} else {
			delete(b.open, d.ID)
		}
		if _, seen := b.visibleAt[d.MessageID]; !seen {
			switch d.State {
			case Held, Dismissed:
				b.heldFirst[d.MessageID] = true
			default:
				b.visibleAt[d.MessageID] = at
			}
		}
		if d.To != "" && !b.isOwner(d.To) {
			if _, ok := b.known[d.To]; !ok {
				b.known[d.To] = ""
			}
		}
		b.touch(prevTo, d.To)
		switch {
		case d.State == Held && (!ok || prevState != Held):
			b.note(Change{Kind: HeldNew, To: d.To, DeliveryID: d.ID})
		case d.State == Moved:
			b.note(Change{Kind: MovedAway, To: prevTo, DeliveryID: d.ID})
		default:
			b.note(Change{Kind: Updated, To: d.To, DeliveryID: d.ID})
		}

	case KindWakeIssued:
		var r wakeRow
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return err
		}
		mode := r.Mode
		if mode == "" {
			mode = ModeWake
		}
		w := &wakeRec{
			Wake: Wake{ID: r.WakeID, To: r.To, Token: r.Token, Reminder: r.Reminder, Mode: mode,
				DeliveryIDs: r.DeliveryIDs, Overflow: r.Overflow},
			path:     r.Path,
			issuedAt: stamp(r.At, ev),
		}
		b.wakes[r.WakeID] = w
		b.tokens[r.Token] = r.WakeID
		for _, id := range r.DeliveryIDs {
			d := b.dels[id]
			if d == nil {
				continue
			}
			if r.Reminder {
				d.Reminded = true
			} else if d.State == Queued {
				d.WakeID = r.WakeID
			}
		}
		if r.Reminder {
			delete(b.reminders, r.To)
		}
		b.touch(r.To)
		b.note(Change{Kind: Updated, To: r.To})

	case KindAlias:
		var r aliasRow
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return err
		}
		b.addAlias(r.Key, r.Alias)

	case KindAgentName:
		var r nameRow
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return err
		}
		if r.Key != "" {
			b.known[r.Key] = strings.ToLower(strings.TrimSpace(r.Name))
		}

	case KindPolicySet:
		var r policyRow
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return err
		}
		b.pol = normPolicy(r.Policy)
		for agent := range b.agentsWithOpen() {
			b.touch(agent)
		}
		b.note(Change{Kind: Updated})

	case KindTurnEngaged:
		var r turnRow
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return err
		}
		r.At = stamp(r.At, ev)
		b.running[r.Agent] = r
		b.touch(r.Agent)

	case KindTurnEnded:
		var r turnRow
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return err
		}
		if cur, ok := b.running[r.Agent]; ok && cur.WakeID == r.WakeID {
			delete(b.running, r.Agent)
		}
		b.touch(r.Agent)

	case KindDecided:
		var r decidedRow
		if err := json.Unmarshal(ev.Payload, &r); err != nil {
			return err
		}
		// Only the person decides a held delivery: a request written by
		// anything but a GUI is ignored and counted.
		if ev.Actor != "user" || !b.isOwner(r.By) {
			b.ignore(ev.DeviceID)
			return nil
		}
		if b.replaying {
			b.requests = append(b.requests, r)
		}
		b.note(Change{Kind: Updated, DeliveryID: r.DeliveryID})

	case store.KindAgentSeen:
		var c presence.AgentCard
		if err := json.Unmarshal(ev.Payload, &c); err != nil {
			return nil // not ours to judge; skip it
		}
		if c.Key != "" {
			b.known[c.Key] = strings.ToLower(strings.TrimSpace(c.Name))
		}
	}
	return nil
}

func normPolicy(p Policy) Policy {
	if p.Mode != Allowlist {
		p.Mode = Everyone
	}
	p.Block = slices.Clone(p.Block)
	p.Allow = slices.Clone(p.Allow)
	if p.Block == nil {
		p.Block = []string{}
	}
	if p.Allow == nil {
		p.Allow = []string{}
	}
	return p
}

// addAlias records alias for key: newest last, at most MaxAliases.
func (b *inbox) addAlias(key, alias string) {
	alias = normAlias(alias)
	if key == "" || alias == "" {
		return
	}
	list := slices.DeleteFunc(b.aliases[key], func(a string) bool { return a == alias })
	list = append(list, alias)
	if len(list) > MaxAliases {
		list = list[len(list)-MaxAliases:]
	}
	b.aliases[key] = list
}

// normAlias lowercases and trims a name; "" when it can't be an alias.
func normAlias(s string) string {
	s = strings.ToLower(strings.TrimSpace(s))
	if s == "" || len([]rune(s)) > MaxAliasLen {
		return ""
	}
	return s
}

func (b *inbox) touch(agents ...string) {
	for _, a := range agents {
		if a != "" {
			b.touched[a] = true
		}
	}
}

func (b *inbox) note(c Change) {
	if !b.replaying {
		b.changes = append(b.changes, c)
	}
}

// unlock computes WakeReady edges for agents the mutation touched, hands
// the changes to subscribers, then releases the lock.
func (b *inbox) unlock() {
	defer b.mu.Unlock()
	for agent := range b.touched {
		now := b.hasSubmit(agent)
		if now && !b.submit[agent] {
			b.changes = append(b.changes, Change{Kind: WakeReady, To: agent})
		}
		b.submit[agent] = now
		app := b.hasAppend(agent)
		if app && !b.appendable[agent] {
			b.changes = append(b.changes, Change{Kind: AppendReady, To: agent})
		}
		b.appendable[agent] = app
	}
	clear(b.touched)
	for _, c := range b.changes {
		for ch := range b.subs {
			select {
			case ch <- c:
			default: // slow reader; it re-reads
			}
		}
	}
	b.changes = b.changes[:0]
}

// agentsWithOpen is every recipient of an open delivery, plus agents with a
// running turn or a due reminder.
func (b *inbox) agentsWithOpen() map[string]bool {
	out := map[string]bool{}
	for _, d := range b.open {
		if d.To != "" {
			out[d.To] = true
		}
	}
	for a := range b.running {
		out[a] = true
	}
	for a := range b.reminders {
		out[a] = true
	}
	return out
}

// hasSubmit: NextWake on the mod path would return a wake for agent right
// now. Queued next_turn items never count: they make no wake by themselves
// (a channel stream picks them up on the Updated change that queued them).
func (b *inbox) hasSubmit(agent string) bool {
	if agent == "" || b.pol.Paused || !b.local(agent) {
		return false
	}
	if w, _ := b.outstanding(agent); w != nil {
		return false
	}
	if len(b.reminders[agent]) > 0 {
		return true
	}
	for _, d := range b.open {
		if d.To == agent && d.State == Queued && d.WakeID == "" && !d.nextTurn() {
			return true
		}
	}
	return false
}

// hasAppend: NextAppend would return an append for agent right now.
func (b *inbox) hasAppend(agent string) bool {
	if agent == "" || b.pol.Paused || !b.local(agent) || b.pendingAppend(agent) != nil {
		return false
	}
	for _, d := range b.open {
		if d.To == agent && d.State == Queued && d.WakeID == "" && d.nextTurn() {
			return true
		}
	}
	return false
}

// pendingAppend is the append issued to agent and not yet confirmed
// delivered (some of its deliveries still Queued in it), or nil.
func (b *inbox) pendingAppend(agent string) *wakeRec {
	var best *wakeRec
	for _, d := range b.open {
		if d.To != agent || d.State != Queued || d.WakeID == "" {
			continue
		}
		if w := b.wakes[d.WakeID]; w != nil && w.isAppend() && (best == nil || w.issuedAt.Before(best.issuedAt)) {
			best = w
		}
	}
	return best
}

// nextTurn: d is a next_turn delivery.
func (d *drec) nextTurn() bool { return d.Mode == ModeNextTurn }

// outstanding is the wake that gates agent: issued and not yet delivered
// (state Queued), else delivered and not yet engaged (state Delivered). A
// delivered wake stops gating after the unengaged limit, so a prompt the
// host dropped can't stall the agent forever.
func (b *inbox) outstanding(agent string) (*wakeRec, State) {
	var delivered *wakeRec
	now := b.now()
	for _, d := range b.open {
		if d.To != agent || d.WakeID == "" {
			continue
		}
		w := b.wakes[d.WakeID]
		if w == nil || !w.gates() {
			continue
		}
		switch d.State {
		case Queued:
			return w, Queued
		case Delivered:
			if now.Sub(d.UpdatedAt) < b.lim.unengaged && (delivered == nil || w.issuedAt.Before(delivered.issuedAt)) {
				delivered = w
			}
		}
	}
	if delivered != nil {
		return delivered, Delivered
	}
	return nil, ""
}

// copyOut returns d with its message filled in.
func (b *inbox) copyOut(d *drec) Delivery {
	out := d.Delivery
	if m := b.msgs[d.MessageID]; m != nil {
		out.Message = *m
	}
	return out
}

func pairKey(a, c string) string {
	if a > c {
		a, c = c, a
	}
	return a + "|" + c
}

// countsForRate: both ends are agents (a person at the GUI and Irori itself
// are exempt).
func (b *inbox) countsForRate(from, to string) bool {
	return from != "" && to != "" && !b.isOwner(from) && !b.isOwner(to) && from != SystemKey && to != SystemKey
}

func (b *inbox) subscribe(ctx context.Context) <-chan Change {
	ch := make(chan Change, subBuffer)
	b.mu.Lock()
	b.subs[ch] = struct{}{}
	b.mu.Unlock()
	go func() {
		<-ctx.Done()
		b.mu.Lock()
		delete(b.subs, ch)
		b.mu.Unlock()
		close(ch)
	}()
	return ch
}

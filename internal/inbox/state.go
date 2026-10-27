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

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// More errors callers can branch on (additions to the contract's set).
var (
	ErrSelf  = errors.New("inbox: that's you; address another agent")
	ErrEmpty = errors.New("inbox: give the message text")
)

// MaxNote caps defer/skip notes and send reasons (runes).
const MaxNote = 500

// subBuffer is each subscriber's channel size.
const subBuffer = 64

// replayKinds are the rows New folds. agent.seen is read only to learn the
// keys and names of agents that have since gone, so they stay addressable.
var replayKinds = []string{
	KindMessageSent, KindDelivery, KindWakeIssued, KindAlias,
	KindPolicySet, KindTurnEngaged, KindTurnEnded, store.KindAgentSeen,
}

// limits are Options' durations with defaults filled in.
type limits struct {
	unengaged, gone, quiet, reattach time.Duration
}

// drec is a delivery plus the bookkeeping the sweeper needs. Message is left
// empty here and filled in when a copy goes out.
type drec struct {
	Delivery
	movedFrom   string    // the delivery this one replaced (a move); moved work never moves on unengaged again
	assignedAt  time.Time // when To last changed
	engagedOnce bool      // the current recipient engaged or deferred it at least once
}

// deliveryRow is the KindDelivery payload: a full snapshot of the delivery's
// mutable fields, so the fold is a replace.
type deliveryRow struct {
	DeliveryID  string     `json:"deliveryId"`
	MessageID   string     `json:"messageId"`
	To          string     `json:"to"`
	RepoKey     string     `json:"repoKey,omitempty"`
	State       State      `json:"state"`
	Hold        HoldReason `json:"hold,omitempty"`
	Via         string     `json:"via,omitempty"`
	WakeID      string     `json:"wakeId,omitempty"`
	Note        string     `json:"note,omitempty"`
	Until       time.Time  `json:"until,omitzero"`
	MovedTo     string     `json:"movedTo,omitempty"`
	MovedFrom   string     `json:"movedFrom,omitempty"`
	Reminded    bool       `json:"reminded,omitempty"`
	EngagedOnce bool       `json:"engagedOnce,omitempty"`
	By          string     `json:"by,omitempty"`
	At          time.Time  `json:"at,omitzero"`
}

// row snapshots d as a deliveryRow, ready to change and append.
func (d *drec) row() deliveryRow {
	return deliveryRow{
		DeliveryID: d.ID, MessageID: d.MessageID, To: d.To, RepoKey: d.RepoKey,
		State: d.State, Hold: d.Hold, Via: d.Via, WakeID: d.WakeID, Note: d.Note,
		Until: d.Until, MovedTo: d.MovedTo, MovedFrom: d.movedFrom,
		Reminded: d.Reminded, EngagedOnce: d.engagedOnce,
	}
}

type wakeRow struct {
	WakeID      string    `json:"wakeId"`
	To          string    `json:"to"`
	Token       string    `json:"token"`
	Reminder    bool      `json:"reminder"`
	DeliveryIDs []string  `json:"deliveryIds"`
	Path        Path      `json:"path,omitempty"`
	At          time.Time `json:"at,omitzero"`
}

type wakeRec struct {
	Wake
	path     Path
	issuedAt time.Time
}

type aliasRow struct {
	Key   string    `json:"key"`
	Alias string    `json:"alias"`
	At    time.Time `json:"at,omitzero"`
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
	dev    string
	now    func() time.Time
	lim    limits
	ids    idGen

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
	reminders map[string][]string // agent -> delivery ids a reminder wake is due for
	submit    map[string]bool     // last hasSubmit per agent, for WakeReady edges
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
	b := &inbox{
		st: st, roster: roster, dev: opts.DeviceID, now: now,
		lim: limits{
			unengaged: pick(opts.UnengagedLimit, UnengagedLimit),
			gone:      pick(opts.GoneGrace, GoneGrace),
			quiet:     pick(opts.QuietLimit, QuietLimit),
			reattach:  pick(opts.ReattachGrace, ReattachGrace),
		},
		msgs: map[string]*Message{}, dels: map[string]*drec{}, open: map[string]*drec{},
		counted: map[string]bool{}, wakes: map[string]*wakeRec{}, tokens: map[string]string{},
		aliases: map[string][]string{}, known: map[string]string{},
		pol:     Policy{Mode: Everyone, Block: []string{}, Allow: []string{}},
		running: map[string]turnRow{}, pairs: map[string][]time.Time{},
		reach: map[string]*reachRec{}, goneSince: map[string]time.Time{},
		reminders: map[string][]string{}, submit: map[string]bool{},
		subs: map[chan Change]struct{}{}, touched: map[string]bool{},
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
	b.replaying = true
	defer func() { b.replaying = false }()
	var after hlc.Timestamp
	for {
		evs, err := b.st.List(ctx, store.Filter{Kinds: replayKinds, AfterHLC: after, Limit: store.MaxListLimit})
		if err != nil {
			return fmt.Errorf("inbox: replay: %w", err)
		}
		for _, ev := range evs {
			if err := b.apply(ev); err != nil {
				return fmt.Errorf("inbox: replay %s %s/%d: %w", ev.Kind, ev.DeviceID, ev.Seq, err)
			}
			after = ev.HLC
		}
		if len(evs) < store.MaxListLimit {
			break
		}
	}
	// Baseline for WakeReady edges; replay itself notifies nobody.
	for agent := range b.agentsWithOpen() {
		b.submit[agent] = b.hasSubmit(agent)
	}
	b.changes = nil
	clear(b.touched)
	return nil
}

// emit appends one row and folds it. Every mutation goes through here, so
// live state and replayed state come from the same code.
func (b *inbox) emit(ctx context.Context, by, kind string, payload any) error {
	ev, err := b.st.Append(ctx, actorFor(by), kind, payload)
	if err != nil {
		return fmt.Errorf("inbox: append %s: %w", kind, err)
	}
	return b.apply(ev)
}

// putDelivery stamps and appends a delivery snapshot.
func (b *inbox) putDelivery(ctx context.Context, r deliveryRow, by string) error {
	r.By = by
	r.At = b.now()
	return b.emit(ctx, by, KindDelivery, r)
}

func actorFor(by string) string {
	switch {
	case by == "" || by == "irori":
		return "irori"
	case strings.HasPrefix(by, "user:"):
		return "user"
	default:
		return "agent:" + by
	}
}

func isUserKey(key string) bool { return strings.HasPrefix(key, "user:") }

// stamp is a row's time: its own "at", else its HLC wall time.
func stamp(at time.Time, ev store.Event) time.Time {
	if !at.IsZero() {
		return at
	}
	return ev.HLC.Wall()
}

// apply folds one row into memory.
func (b *inbox) apply(ev store.Event) error {
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
		if m.From.Key != "" && !m.From.IsUser() {
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
			b.dels[r.DeliveryID] = d
			b.order = append(b.order, r.DeliveryID)
			d.assignedAt = at
			if !b.counted[r.MessageID] {
				b.counted[r.MessageID] = true
				if m := b.msgs[r.MessageID]; m != nil && r.MovedFrom == "" && countsForRate(m.From.Key, r.To) {
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
		d.MessageID, d.To, d.RepoKey = r.MessageID, r.To, r.RepoKey
		d.State, d.Hold, d.Via, d.WakeID = r.State, r.Hold, r.Via, r.WakeID
		d.Note, d.Until, d.MovedTo, d.Reminded = r.Note, r.Until, r.MovedTo, r.Reminded
		d.movedFrom, d.engagedOnce = r.MovedFrom, r.EngagedOnce
		d.UpdatedAt = at
		if d.State.Open() {
			b.open[d.ID] = d
		} else {
			delete(b.open, d.ID)
		}
		if d.To != "" && !isUserKey(d.To) {
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
		w := &wakeRec{
			Wake:     Wake{ID: r.WakeID, To: r.To, Token: r.Token, Reminder: r.Reminder, DeliveryIDs: r.DeliveryIDs},
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

// hasSubmit: NextWake would return a wake for agent right now.
func (b *inbox) hasSubmit(agent string) bool {
	if agent == "" || b.pol.Paused {
		return false
	}
	if w, _ := b.outstanding(agent); w != nil {
		return false
	}
	if len(b.reminders[agent]) > 0 {
		return true
	}
	for _, d := range b.open {
		if d.To == agent && d.State == Queued && d.WakeID == "" {
			return true
		}
	}
	return false
}

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
		if w == nil || w.Reminder {
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

// countsForRate: both ends are agents (a person at the GUI is exempt).
func countsForRate(from, to string) bool {
	return from != "" && to != "" && !isUserKey(from) && !isUserKey(to)
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

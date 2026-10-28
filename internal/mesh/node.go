package mesh

import (
	"context"
	"crypto/ed25519"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/store"
)

// MinPeerSchema is the oldest store schema a peer may run and still sync
// (the first v0.4 schema). An older peer is refused with reason "version".
const MinPeerSchema = 3

// Envelope types on the mesh topic (§4.2). go-room owns rtc.*.
const (
	MsgPresence = "presence"
	MsgLeave    = "leave"
)

// Presence is the presence envelope's body. It never logs a row.
type Presence struct {
	DeviceID    string `json:"deviceId"`
	Epoch       int    `json:"epoch"`
	CoreVersion string `json:"coreVersion"`
}

// Intervals are the mesh's timers; zero values take the design's defaults.
// Tests shorten them.
type Intervals struct {
	Presence   time.Duration // presence on the mesh topic (30 s)
	Ping       time.Duration // ping on each connection (15 s)
	Vector     time.Duration // unsolicited vector, and retrying stalled devices (30 s)
	BackoffMin time.Duration // reconnect after a drop: from 1 s
	BackoffMax time.Duration // up to 60 s
}

func (iv Intervals) withDefaults() Intervals {
	def := func(d *time.Duration, v time.Duration) {
		if *d <= 0 {
			*d = v
		}
	}
	def(&iv.Presence, 30*time.Second)
	def(&iv.Ping, 15*time.Second)
	def(&iv.Vector, 30*time.Second)
	def(&iv.BackoffMin, time.Second)
	def(&iv.BackoffMax, 60*time.Second)
	return iv
}

// Held-row caps per peer (§5.2): rows refused only for "no cert yet" wait in
// memory up to these, and past them are dropped and asked for again later.
const (
	HeldRowsCap  = 2000
	HeldBytesCap = 8 << 20
)

// Liveness windows (§6.2).
const (
	ConnectedWithin = 45 * time.Second
	RecentWithin    = 10 * time.Minute
)

// Options configure a Node.
type Options struct {
	Store   *store.Store
	Ident   *identity.Identity
	Chain   *identity.Chain
	Net     Transport
	Version string
	// Intervals shortens the timers (tests).
	Intervals Intervals
	// TrustAnyPeer turns off the handshake's cert check and hello's key
	// check, so an uncertified peer connects. Tests only: it proves Insert
	// and the folds stand alone (§8). Secrets still go only to certified
	// devices.
	TrustAnyPeer bool
	Logf         func(format string, args ...any)
	Now          func() time.Time
}

// Node is the Core side of the mesh: it starts the net layer once the
// person has a second device, keeps a room per open epoch, and runs sync
// on every connection.
type Node struct {
	o    Options
	iv   Intervals
	self string
	now  func() time.Time
	logf func(string, ...any)
	kick chan struct{}

	roomMu sync.Mutex // serializes ensureRooms: one Join per epoch

	mu        sync.Mutex
	ctx       context.Context
	state     string // StateOff, StateOn, StateRemoved, StateError
	detail    string
	rooms     map[int]Room
	sessions  map[*session]struct{}
	known     map[string]*peerInfo
	stopped   map[string]string // device -> why its stream stopped here
	ahead     map[string]time.Time
	pending   []pendingSecret
	restoring string // the peer this device's own rows are coming back from
	restoreTo int64  // and up to which seq
	retry     map[string]*retryState
	wg        sync.WaitGroup
}

// Net states.
const (
	StateOff     = "off"     // no other device: nothing is open
	StateOn      = "on"      // the net layer runs
	StateRemoved = "removed" // this device was revoked: it stops syncing
	StateError   = "error"   // no topic could be joined (Detail says why); retried, and the app works alone
)

type peerInfo struct {
	epoch        int
	lastPing     time.Time
	lastSync     time.Time
	lastPresence time.Time
	skew         time.Duration
	refused      int
	schemaOld    bool
}

type pendingSecret struct {
	epoch int
	seed  []byte
}

type retryState struct {
	backoff time.Duration
	timer   *time.Timer
}

// New returns a Node; Run starts it.
func New(o Options) *Node {
	n := &Node{
		o:        o,
		iv:       o.Intervals.withDefaults(),
		self:     o.Ident.Device.ID,
		now:      o.Now,
		logf:     o.Logf,
		kick:     make(chan struct{}, 1),
		state:    StateOff,
		rooms:    map[int]Room{},
		sessions: map[*session]struct{}{},
		known:    map[string]*peerInfo{},
		stopped:  map[string]string{},
		ahead:    map[string]time.Time{},
		retry:    map[string]*retryState{},
	}
	if n.now == nil {
		n.now = time.Now
	}
	if n.logf == nil {
		n.logf = func(string, ...any) {}
	}
	return n
}

// Kick re-reads the cert fold now: a pairing that Learned rows, or a test.
func (n *Node) Kick() {
	select {
	case n.kick <- struct{}{}:
	default:
	}
}

// HasPeer reports whether the person has another certified, unrevoked
// device: the condition for the net layer to start (§4.5).
func (n *Node) HasPeer() bool {
	for _, d := range n.o.Chain.Devices() {
		if d.Cert.DeviceID != n.self && !d.Revoked {
			return true
		}
	}
	return false
}

// Run waits until the person has a second device, then runs the net layer
// until ctx ends. A device on its own opens nothing. It returns ctx's
// error, after leaving every room.
func (n *Node) Run(ctx context.Context) error {
	sub := n.o.Store.Subscribe(ctx)
	n.mu.Lock()
	n.ctx = ctx
	n.mu.Unlock()
	if forks, err := n.o.Store.Forks(ctx); err == nil {
		n.mu.Lock()
		for _, f := range forks {
			n.stopped[f.Stored.DeviceID] = "signed two different histories"
		}
		n.mu.Unlock()
	}
	for !n.HasPeer() || n.isRemoved() {
		if n.isRemoved() {
			n.setState(StateRemoved, "")
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case _, ok := <-sub:
			if !ok {
				return ctx.Err()
			}
		case <-n.kick:
		}
	}
	n.setState(StateOn, "")
	n.personChanged()
	pres := time.NewTicker(n.iv.Presence)
	defer pres.Stop()
	defer n.shutdown()
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case ev, ok := <-sub:
			if !ok {
				return ctx.Err()
			}
			n.wakeAll()
			if ev.Seq == 0 || store.IsPersonKind(ev.Kind) {
				n.personChanged()
			}
		case <-n.kick:
			n.personChanged()
		case <-pres.C:
			n.ensureRooms() // retries a failed join
			n.announce()
		}
	}
}

func (n *Node) setState(state, detail string) {
	n.mu.Lock()
	n.state, n.detail = state, detail
	n.mu.Unlock()
}

func (n *Node) isRemoved() bool {
	_, revoked := n.o.Chain.Cut(n.self)
	return revoked
}

// certified reports a device with a valid, unrevoked cert for the person.
func (n *Node) certified(dev string) bool {
	if dev == n.self {
		return false
	}
	if _, ok := n.o.Chain.Cert(dev); !ok {
		return false
	}
	_, revoked := n.o.Chain.Cut(dev)
	return !revoked
}

// accept is every room's Accept: only a certified, unrevoked device whose
// handshake proved the key its cert names (§4.3).
func (n *Node) accept(peer string, pub ed25519.PublicKey) error {
	if peer == n.self {
		return ErrRefused
	}
	if n.o.TrustAnyPeer {
		return nil
	}
	cert, ok := n.o.Chain.Cert(peer)
	if !ok || !n.certified(peer) || hex.EncodeToString(pub) != cert.DevicePub {
		return ErrRefused
	}
	return nil
}

// allow is the Allow of the room for epoch: certified devices only, and on
// a previous epoch's topic only devices still behind (§4.2, Rotation).
func (n *Node) allow(epoch int, peer string) bool {
	if peer == n.self || n.isRemoved() {
		return false
	}
	if !n.o.TrustAnyPeer && !n.certified(peer) {
		return false
	}
	cur := n.o.Ident.Current().Epoch
	if epoch < cur {
		n.mu.Lock()
		k := n.known[peer]
		ahead := k != nil && k.epoch >= cur
		n.mu.Unlock()
		if ahead {
			return false
		}
	}
	return true
}

func (n *Node) info(dev string) *peerInfo {
	k := n.known[dev]
	if k == nil {
		k = &peerInfo{}
		n.known[dev] = k
	}
	return k
}

// personChanged re-reads the cert fold: a removed device stops, revoked
// peers are dropped, a waiting person.secret may now be adopted, the epoch
// rooms are opened or closed and held rows are retried.
func (n *Node) personChanged() {
	if n.isRemoved() {
		n.setState(StateRemoved, "")
		n.closeRooms()
		return
	}
	n.adoptPending()
	cur := n.o.Ident.Current().Epoch
	for _, s := range n.sessionList() {
		peer := s.conn.ID()
		switch {
		case !n.o.TrustAnyPeer && !n.certified(peer):
			s.conn.Close("revoked")
		case s.epoch < cur && !n.allow(s.epoch, peer):
			s.conn.Close("epoch") // both have the new epoch: the new topic only
		}
	}
	n.ensureRooms()
	for _, s := range n.sessionList() {
		s.retryHeld()
	}
}

// ensureRooms keeps the current epoch's room open, and the previous
// epoch's while any certified device hasn't said hello with the current
// one (§4.2).
func (n *Node) ensureRooms() {
	n.roomMu.Lock()
	defer n.roomMu.Unlock()
	n.mu.Lock()
	if n.state != StateOn || n.ctx == nil || n.ctx.Err() != nil {
		n.mu.Unlock()
		return
	}
	n.mu.Unlock()
	cur := n.o.Ident.Current().Epoch
	want := map[int]bool{cur: true}
	if prev := n.o.Ident.PrevEpoch(); prev > 0 && n.someoneBehind(cur) {
		want[prev] = true
	}
	n.mu.Lock()
	var closing []Room
	for e, r := range n.rooms {
		if !want[e] {
			closing = append(closing, r)
			delete(n.rooms, e)
		}
	}
	var opening []int
	for e := range want {
		if n.rooms[e] == nil {
			opening = append(opening, e)
		}
	}
	n.mu.Unlock()
	for _, r := range closing {
		_ = r.Send("", MsgLeave, struct{}{})
		r.Close()
	}
	sort.Ints(opening)
	for _, e := range opening {
		n.join(e)
	}
}

func (n *Node) someoneBehind(cur int) bool {
	n.mu.Lock()
	defer n.mu.Unlock()
	for _, d := range n.o.Chain.Devices() {
		id := d.Cert.DeviceID
		if id == n.self || d.Revoked {
			continue
		}
		if k := n.known[id]; k == nil || k.epoch < cur {
			return true
		}
	}
	return false
}

func (n *Node) join(epoch int) {
	seed, ok := n.o.Ident.PersonSeed(epoch)
	if !ok {
		return
	}
	s := MeshSecret(seed, n.o.Ident.Current().ID, epoch)
	r, err := n.o.Net.Join(s, RoomConfig{
		Self:      n.self,
		Pub:       n.o.Ident.DevicePub(),
		Sign:      n.o.Ident.Sign,
		Accept:    n.accept,
		Allow:     func(peer string) bool { return n.allow(epoch, peer) },
		OnMsg:     func(env Envelope) { n.onMsg(epoch, env) },
		OnConnect: func() { n.announceIn(epoch) },
		OnPeer:    func(c Conn) { n.onPeer(epoch, c) },
	})
	if err != nil {
		// Retried on the next presence tick; the app works alone meanwhile.
		n.logf("mesh: join epoch %d: %v", epoch, err)
		n.mu.Lock()
		n.detail = "Can't reach the signaling servers: " + err.Error()
		n.mu.Unlock()
		return
	}
	n.mu.Lock()
	if n.rooms[epoch] != nil || n.state != StateOn {
		n.mu.Unlock()
		r.Close()
		return
	}
	n.rooms[epoch] = r
	n.detail = ""
	n.mu.Unlock()
	n.announceIn(epoch)
}

func (n *Node) room(epoch int) Room {
	n.mu.Lock()
	defer n.mu.Unlock()
	return n.rooms[epoch]
}

func (n *Node) presence() Presence {
	return Presence{DeviceID: n.self, Epoch: n.o.Ident.Current().Epoch, CoreVersion: n.o.Version}
}

func (n *Node) announceIn(epoch int) {
	if r := n.room(epoch); r != nil {
		_ = r.Send("", MsgPresence, n.presence())
	}
}

func (n *Node) announce() {
	n.mu.Lock()
	var eps []int
	for e := range n.rooms {
		eps = append(eps, e)
	}
	n.mu.Unlock()
	for _, e := range eps {
		n.announceIn(e)
	}
}

// onMsg handles presence: each one from an allowed device is a Connect,
// which is idempotent and lets the transport pick who offers (§4.2).
func (n *Node) onMsg(epoch int, env Envelope) {
	switch env.Type {
	case MsgPresence:
		n.mu.Lock()
		n.info(env.From).lastPresence = n.now()
		n.mu.Unlock()
		if n.allow(epoch, env.From) {
			if r := n.room(epoch); r != nil {
				r.Connect(env.From)
			}
		}
	case MsgLeave:
		n.mu.Lock()
		if k := n.known[env.From]; k != nil {
			k.lastPresence = time.Time{}
		}
		n.mu.Unlock()
	}
}

func (n *Node) onPeer(epoch int, c Conn) {
	n.mu.Lock()
	ctx := n.ctx
	if ctx == nil || ctx.Err() != nil || n.state != StateOn || c.ID() == n.self {
		n.mu.Unlock()
		c.Close("not running")
		return
	}
	s := newSession(n, epoch, c)
	n.sessions[s] = struct{}{}
	n.wg.Add(1)
	n.mu.Unlock()
	go func() {
		defer n.wg.Done()
		s.run(ctx)
		n.mu.Lock()
		delete(n.sessions, s)
		n.mu.Unlock()
		n.scheduleReconnect(c.ID(), c.Err())
	}()
}

// scheduleReconnect tries peer again after a drop, 1 s, 2 s, 4 s ... up to
// 60 s apart, on top of the presence-driven Connect (§4.5). A local close
// or a refusal is not retried.
func (n *Node) scheduleReconnect(peer string, why error) {
	n.mu.Lock()
	defer n.mu.Unlock()
	if n.ctx == nil || n.ctx.Err() != nil || n.state != StateOn {
		return
	}
	rs := n.retry[peer]
	if rs == nil {
		rs = &retryState{}
		n.retry[peer] = rs
	}
	if rs.timer != nil {
		return
	}
	if rs.backoff == 0 {
		rs.backoff = n.iv.BackoffMin
	} else {
		rs.backoff = min(rs.backoff*2, n.iv.BackoffMax)
	}
	rs.timer = time.AfterFunc(rs.backoff, func() {
		n.mu.Lock()
		rs.timer = nil
		var rooms []struct {
			e int
			r Room
		}
		for e, r := range n.rooms {
			rooms = append(rooms, struct {
				e int
				r Room
			}{e, r})
		}
		n.mu.Unlock()
		if n.connectedTo(peer) {
			return
		}
		for _, x := range rooms {
			if n.allow(x.e, peer) {
				x.r.Connect(peer)
			}
		}
		if !n.connectedTo(peer) && n.allowAny(peer) {
			n.scheduleReconnect(peer, nil)
		}
	})
}

func (n *Node) allowAny(peer string) bool {
	n.mu.Lock()
	var eps []int
	for e := range n.rooms {
		eps = append(eps, e)
	}
	n.mu.Unlock()
	for _, e := range eps {
		if n.allow(e, peer) {
			return true
		}
	}
	return false
}

func (n *Node) connectedTo(peer string) bool {
	for _, s := range n.sessionList() {
		if s.conn.ID() == peer && s.helloDone() {
			return true
		}
	}
	return false
}

// connected marks peer reachable again: its backoff starts over.
func (n *Node) connected(peer string) {
	n.mu.Lock()
	defer n.mu.Unlock()
	if rs := n.retry[peer]; rs != nil {
		if rs.timer != nil {
			rs.timer.Stop()
		}
		delete(n.retry, peer)
	}
}

func (n *Node) sessionList() []*session {
	n.mu.Lock()
	defer n.mu.Unlock()
	out := make([]*session, 0, len(n.sessions))
	for s := range n.sessions {
		out = append(out, s)
	}
	return out
}

func (n *Node) wakeAll() {
	for _, s := range n.sessionList() {
		s.poke()
	}
}

func (n *Node) closeRooms() {
	n.roomMu.Lock()
	defer n.roomMu.Unlock()
	n.mu.Lock()
	rooms := n.rooms
	n.rooms = map[int]Room{}
	for _, rs := range n.retry {
		if rs.timer != nil {
			rs.timer.Stop()
		}
	}
	n.retry = map[string]*retryState{}
	n.mu.Unlock()
	for _, r := range rooms {
		_ = r.Send("", MsgLeave, struct{}{})
		r.Close()
	}
	for _, s := range n.sessionList() {
		s.conn.Close("leaving")
	}
}

// shutdown leaves every room and waits for the sessions to end, so nothing
// writes to the store after Run returns.
func (n *Node) shutdown() {
	n.mu.Lock()
	if n.state == StateOn {
		n.state = StateOff
	}
	n.mu.Unlock()
	n.closeRooms()
	done := make(chan struct{})
	go func() { n.wg.Wait(); close(done) }()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		n.logf("mesh: sessions did not stop in time")
	}
}

// hello is this device's first frame on a connection.
func (n *Node) hello(ctx context.Context, peer string) Frame {
	v, err := n.o.Store.Vector(ctx)
	if err != nil {
		v = store.Vector{}
	}
	cur := n.o.Ident.Current()
	f := Frame{Type: FrameHello, V: Protocol, DeviceID: n.self, PersonID: cur.ID, Epoch: cur.Epoch,
		CoreVersion: n.o.Version, Schema: store.SchemaVersion(), WallMs: n.now().UnixMilli(), Vector: v}
	if ev, ok := n.rowAt(ctx, peer, v[peer]); ok {
		f.Top = store.RowHash(ev)
	}
	return f
}

// rowAt is dev's row at seq, if held.
func (n *Node) rowAt(ctx context.Context, dev string, seq int64) (store.Event, bool) {
	if seq < 1 {
		return store.Event{}, false
	}
	evs, err := n.o.Store.List(ctx, store.Filter{DeviceID: dev, AfterSeq: seq - 1, Limit: 1})
	if err != nil || len(evs) != 1 || evs[0].Seq != seq {
		return store.Event{}, false
	}
	return evs[0], true
}

// alive records a ping or pong from peer (liveness, §6.2).
func (n *Node) alive(peer string) {
	n.mu.Lock()
	defer n.mu.Unlock()
	n.info(peer).lastPing = n.now()
}

// synced records rows applied from peer.
func (n *Node) synced(peer string) {
	n.mu.Lock()
	defer n.mu.Unlock()
	k := n.info(peer)
	k.lastSync = n.now()
	k.lastPing = k.lastSync
}

// stop marks dev's stream stopped on this device: it signed two histories
// or its clock went backwards (§5.2, §5.3).
func (n *Node) stop(dev, why string) {
	n.mu.Lock()
	defer n.mu.Unlock()
	if _, ok := n.stopped[dev]; !ok {
		n.stopped[dev] = why
		n.logf("mesh: %s's stream stopped: %s", dev, why)
	}
}

func (n *Node) isStopped(dev string) bool {
	n.mu.Lock()
	defer n.mu.Unlock()
	_, ok := n.stopped[dev]
	return ok
}

// sendSecret hands s's peer, which said hello with an older epoch, the
// current person secret (§3, Revoke). Only to certified, unrevoked devices.
func (n *Node) sendSecret(s *session, peerEpoch int) {
	peer := s.conn.ID()
	if !n.certified(peer) {
		return
	}
	cur := n.o.Ident.Current()
	if peerEpoch >= cur.Epoch {
		return
	}
	old, ok := n.o.Ident.PersonSeed(peerEpoch)
	if !ok {
		n.logf("mesh: %s is on epoch %d, which this device no longer holds", peer, peerEpoch)
		return
	}
	seed, _ := n.o.Ident.PersonSeed(cur.Epoch)
	sealed, err := sealSecret(old, seed, cur.ID, cur.Epoch, peer)
	if err != nil {
		n.logf("mesh: seal person secret: %v", err)
		return
	}
	s.control(Frame{Type: FramePersonSecret, Epoch: cur.Epoch, Sealed: sealed})
}

// takeSecret opens a person.secret frame. It is adopted once the matching
// person.key.rotated row is held (it may still be on its way).
func (n *Node) takeSecret(f Frame) {
	cur := n.o.Ident.Current()
	if f.Epoch <= cur.Epoch {
		return
	}
	old, _ := n.o.Ident.PersonSeed(cur.Epoch)
	seed, err := openSecret(old, f.Sealed, cur.ID, f.Epoch, n.self)
	if err != nil {
		n.logf("mesh: %v", err)
		return
	}
	n.mu.Lock()
	n.pending = append(n.pending, pendingSecret{epoch: f.Epoch, seed: seed})
	n.mu.Unlock()
	n.Kick()
}

func (n *Node) adoptPending() {
	n.mu.Lock()
	pend := n.pending
	n.pending = nil
	n.mu.Unlock()
	var keep []pendingSecret
	for _, p := range pend {
		if p.epoch <= n.o.Ident.Current().Epoch {
			continue
		}
		pub, ok := n.rotationPub(p.epoch)
		if !ok {
			keep = append(keep, p)
			continue
		}
		if err := n.o.Ident.AdoptSecret(p.epoch, p.seed, pub); err != nil {
			n.logf("mesh: adopt person secret: %v", err)
			continue
		}
		n.logf("mesh: adopted person epoch %d", p.epoch)
	}
	if len(keep) > 0 {
		n.mu.Lock()
		n.pending = append(n.pending, keep...)
		n.mu.Unlock()
	}
}

// rotationPub finds the person key that a held person.key.rotated row names
// for epoch. Held rows are verified, so its writer was certified.
func (n *Node) rotationPub(epoch int) (string, bool) {
	ctx := context.Background()
	f := store.Filter{Kinds: []string{store.KindPersonKeyRotated}, Limit: store.MaxListLimit}
	pid := n.o.Ident.Current().ID
	for {
		evs, err := n.o.Store.List(ctx, f)
		if err != nil || len(evs) == 0 {
			return "", false
		}
		for _, ev := range evs {
			var r identity.KeyRotated
			if json.Unmarshal(ev.Payload, &r) == nil && r.PersonID == pid && r.Epoch == epoch && n.everCertified(ev.DeviceID) {
				return r.PersonPub, true
			}
		}
		f.After = evs[len(evs)-1].Cursor()
	}
}

func (n *Node) everCertified(dev string) bool {
	_, ok := n.o.Chain.Cert(dev)
	return ok || dev == n.self
}

// ErrNotPeer: Revoke was given this device or one that isn't certified.
var ErrNotPeer = errors.New("mesh: not one of this person's other devices")

// Revoke removes dev (§3, Revoke): it writes person.device.revoked with the
// cut at the highest of dev's rows held here, rotates the person key, drops
// dev's connections and hands the new secret to every connected device.
// Devices offline now get it when they next connect, on the previous
// epoch's topic, which stays open until then.
func (n *Node) Revoke(ctx context.Context, dev, reason string) error {
	if dev == n.self {
		return ErrNotPeer
	}
	if _, ok := n.o.Chain.Cert(dev); !ok {
		return ErrNotPeer
	}
	if _, revoked := n.o.Chain.Cut(dev); revoked {
		return nil
	}
	vec, err := n.o.Store.Vector(ctx)
	if err != nil {
		return err
	}
	cut, hash := vec[dev], ""
	if cut > 0 {
		evs, err := n.o.Store.List(ctx, store.Filter{DeviceID: dev, AfterSeq: cut - 1, Limit: 1})
		if err != nil {
			return err
		}
		if len(evs) == 1 && evs[0].Seq == cut {
			hash = store.RowHash(evs[0])
		}
	}
	rev, err := n.o.Ident.RevokeOf(dev, cut, hash, reason, n.now())
	if err != nil {
		return err
	}
	if _, err := n.o.Store.Append(ctx, "irori", store.KindPersonDeviceRevoked, rev); err != nil {
		return err
	}
	rot, err := n.o.Ident.Rotate(n.now())
	if err != nil {
		return fmt.Errorf("mesh: rotate the person key: %w", err)
	}
	if _, err := n.o.Store.Append(ctx, "irori", store.KindPersonKeyRotated, rot); err != nil {
		return fmt.Errorf("mesh: record the rotation: %w", err)
	}
	n.personChanged()
	for _, s := range n.sessionList() {
		if s.conn.ID() != dev {
			n.sendSecret(s, s.peerEpoch())
		}
	}
	return nil
}

// PeerState is one other device as the mesh sees it (§6.2, §9). It is kept
// in memory and never logged.
type PeerState struct {
	DeviceID string        `json:"deviceId"`
	Label    string        `json:"label"`
	Role     string        `json:"role"`
	State    string        `json:"state"` // connected, recent, offline
	Revoked  bool          `json:"revoked,omitempty"`
	Epoch    int           `json:"epoch,omitempty"`
	Path     string        `json:"path,omitempty"`
	RTT      time.Duration `json:"rtt,omitempty"`
	LastPing time.Time     `json:"lastPing,omitzero"`
	LastSync time.Time     `json:"lastSync,omitzero"`
	Skew     time.Duration `json:"skew,omitempty"`
	Refused  int           `json:"refused,omitempty"`
	Held     int           `json:"held,omitempty"`
	Stopped  string        `json:"stopped,omitempty"` // why its stream stopped here
	Ahead    bool          `json:"ahead,omitempty"`   // its rows wait for wall time
	Old      bool          `json:"old,omitempty"`     // its schema is too old to sync
}

// Status is the net layer's state and every other certified device.
type Status struct {
	State     string      `json:"state"`
	Detail    string      `json:"detail,omitempty"`
	Epoch     int         `json:"epoch"`
	Topics    int         `json:"topics"`
	Restoring string      `json:"restoring,omitempty"` // own rows coming back from this device
	Peers     []PeerState `json:"peers"`
}

// Status reports the mesh for Devices and GET /v1/devices?detail=1.
func (n *Node) Status() Status {
	now := n.now()
	sessions := n.sessionList()
	n.mu.Lock()
	st := Status{State: n.state, Detail: n.detail, Epoch: n.o.Ident.Current().Epoch, Topics: len(n.rooms), Restoring: n.restoring}
	if st.State == StateOn && st.Topics == 0 && st.Detail != "" {
		st.State = StateError
	}
	known := map[string]peerInfo{}
	for id, k := range n.known {
		known[id] = *k
	}
	stopped := map[string]string{}
	for d, w := range n.stopped {
		stopped[d] = w
	}
	ahead := map[string]bool{}
	for d, t := range n.ahead {
		if now.Sub(t) < 2*n.iv.Vector {
			ahead[d] = true
		}
	}
	n.mu.Unlock()
	if st.Restoring != "" {
		if v, err := n.o.Store.Vector(context.Background()); err == nil {
			if !n.restoringNow(v) {
				st.Restoring = ""
			}
		}
	}
	for _, d := range n.o.Chain.Devices() {
		id := d.Cert.DeviceID
		if id == n.self {
			continue
		}
		k := known[id]
		ps := PeerState{DeviceID: id, Label: d.Cert.Label, Role: d.Cert.Role, Revoked: d.Revoked, Epoch: k.epoch,
			LastPing: k.lastPing, LastSync: k.lastSync, Skew: k.skew, Refused: k.refused,
			Stopped: stopped[id], Ahead: ahead[id], Old: k.schemaOld, State: "offline"}
		for _, s := range sessions {
			if s.conn.ID() == id && s.helloDone() {
				ps.Path, ps.RTT = s.conn.Path(), s.conn.RTT()
				ps.Held += s.heldCount()
			}
		}
		switch {
		case !k.lastPing.IsZero() && now.Sub(k.lastPing) < ConnectedWithin && ps.Path != "":
			ps.State = "connected"
		case !k.lastSync.IsZero() && now.Sub(k.lastSync) < RecentWithin:
			ps.State = "recent"
		}
		st.Peers = append(st.Peers, ps)
	}
	return st
}

// Liveness is one other device's state (§6.2): "connected" when a ping
// arrived within 45 s on a live connection, "recent" when it synced within
// 10 minutes, else "offline". It is kept in memory and never logged.
func (n *Node) Liveness(dev string) string {
	now := n.now()
	up := false
	for _, s := range n.sessionList() {
		if s.conn.ID() == dev && s.helloDone() {
			up = true
			break
		}
	}
	n.mu.Lock()
	k := n.known[dev]
	var ping, sync time.Time
	if k != nil {
		ping, sync = k.lastPing, k.lastSync
	}
	n.mu.Unlock()
	switch {
	case up && !ping.IsZero() && now.Sub(ping) < ConnectedWithin:
		return "connected"
	case !sync.IsZero() && now.Sub(sync) < RecentWithin:
		return "recent"
	}
	return "offline"
}

func (n *Node) restoringNow(v store.Vector) bool {
	n.mu.Lock()
	defer n.mu.Unlock()
	return n.restoreTo > v[n.self]
}

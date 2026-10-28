package mesh

import (
	"context"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"math"
	"sort"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/store"
)

// session is sync on one connection (§5.2). Its state is keyed on the Conn,
// not the device id: a replaced connection is a new session.
//
// Two goroutines run it: the reader applies frames as they arrive, and the
// writer sends hello, then control frames (vector, pong, person.secret)
// ahead of rows, one rows batch at a time: the next goes once the peer's
// ack vector shows the last one applied.
type session struct {
	n     *Node
	epoch int // the room's epoch
	conn  Conn
	ctl   chan Frame
	wake  chan struct{}

	mu        sync.Mutex
	helloRecv bool
	pEpoch    int
	pPub      string // the person key its hello carried
	sentKey   string // the epoch/key whose person.secret went to it
	peerVec   store.Vector
	sentLoose bool
	inflight  bool
	sentAt    time.Time
	extent    map[string][2]int64 // the batch in flight: device -> first, last seq
	stuck     map[string]bool     // devices the peer didn't take; skipped until the next tick
	held      map[string]map[int64]store.Event
	heldRows  int
	heldBytes int
	// ownCheck is the seq of this device's last own row, asked of a peer
	// that holds more of them (row.at): until the peer's copy matches, this
	// device's own rows from it are held, not inserted (§5.2).
	ownCheck int64
	// Blob streams being served; run waits for them, so nothing reads the
	// store after the session ends.
	serving sync.WaitGroup
	ended   bool
}

func newSession(n *Node, epoch int, c Conn) *session {
	return &session{n: n, epoch: epoch, conn: c, ctl: make(chan Frame, 64), wake: make(chan struct{}, 1),
		peerVec: store.Vector{}, stuck: map[string]bool{}}
}

func (s *session) poke() {
	select {
	case s.wake <- struct{}{}:
	default:
	}
}

// control queues a frame for the writer. A full queue drops it: vectors and
// pongs repeat, and the queue only fills when the connection is stuck.
func (s *session) control(f Frame) {
	select {
	case s.ctl <- f:
	default:
		s.n.logf("mesh: %s: control queue full, dropped %s", s.conn.ID(), f.Type)
	}
}

func (s *session) helloDone() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.helloRecv
}

// peerKey is the epoch and person key the peer's hello carried.
func (s *session) peerKey() (int, string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.pEpoch, s.pPub
}

// markSecret records that cur's secret goes to the peer on this
// connection; false if it already went.
func (s *session) markSecret(cur identity.Person) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	k := fmt.Sprintf("%d/%s", cur.Epoch, cur.Pub)
	if s.sentKey == k {
		return false
	}
	s.sentKey = k
	return true
}

func (s *session) heldCount() int {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.heldRows
}

func (s *session) run(parent context.Context) {
	ctx, cancel := context.WithCancel(parent)
	defer cancel()
	// Blob streams the peer opens (§4.3); each is served on its own, and
	// closed when the session ends.
	peer := s.conn.ID()
	s.conn.OnStream(func(label string, st io.ReadWriteCloser) {
		s.mu.Lock()
		if s.ended {
			s.mu.Unlock()
			st.Close()
			return
		}
		s.serving.Add(1)
		s.mu.Unlock()
		defer s.serving.Done()
		stop := context.AfterFunc(ctx, func() { st.Close() })
		defer stop()
		s.n.serveStream(peer, label, st)
	})
	defer func() {
		cancel()
		s.mu.Lock()
		s.ended = true
		s.mu.Unlock()
		s.serving.Wait()
	}()
	go func() {
		select {
		case <-s.conn.Done():
		case <-ctx.Done():
			if parent.Err() != nil {
				s.conn.Close("shutting down")
			}
		}
		cancel()
	}()
	readDone := make(chan struct{})
	go func() {
		defer close(readDone)
		defer cancel()
		s.read(ctx)
	}()
	s.write(ctx)
	cancel()
	<-readDone
}

func (s *session) send(ctx context.Context, f Frame) error {
	b, err := f.Encode()
	if err != nil {
		return err
	}
	return s.conn.Send(ctx, b)
}

func (s *session) write(ctx context.Context) {
	if err := s.send(ctx, s.n.hello(ctx, s.conn.ID())); err != nil {
		s.conn.Close("hello")
		return
	}
	ping := time.NewTicker(s.n.iv.Ping)
	defer ping.Stop()
	tick := time.NewTicker(s.n.iv.Vector)
	defer tick.Stop()
	for {
		select {
		case f := <-s.ctl:
			if s.send(ctx, f) != nil {
				return
			}
			continue
		default:
		}
		frames := s.next(ctx)
		for _, f := range frames {
			if s.send(ctx, f) != nil {
				return
			}
		}
		if len(frames) > 0 {
			continue
		}
		select {
		case <-ctx.Done():
			return
		case f := <-s.ctl:
			if s.send(ctx, f) != nil {
				return
			}
		case <-s.wake:
		case <-ping.C:
			if s.send(ctx, Frame{Type: FramePing, T: s.n.now().UnixMilli()}) != nil {
				return
			}
		case <-tick.C:
			if s.tick(ctx) != nil {
				return
			}
		}
	}
}

// tick sends the periodic vector, retries devices the peer didn't take
// (rows waiting for wall time, or dropped past the held cap) and gives up
// on an ack that never came.
func (s *session) tick(ctx context.Context) error {
	s.mu.Lock()
	s.stuck = map[string]bool{}
	if s.inflight && s.n.now().Sub(s.sentAt) > 2*s.n.iv.Vector {
		s.inflight, s.extent = false, nil
	}
	s.mu.Unlock()
	v, err := s.n.o.Store.Vector(ctx)
	if err != nil {
		return nil
	}
	return s.send(ctx, Frame{Type: FrameVector, Vector: v})
}

// next is what the writer sends now: the loose rows once after hello, then
// the next batch of Missing(peer's vector) when none is in flight.
func (s *session) next(ctx context.Context) []Frame {
	s.mu.Lock()
	if !s.helloRecv {
		s.mu.Unlock()
		return nil
	}
	if !s.sentLoose {
		s.sentLoose = true
		v := copyVec(s.peerVec)
		s.mu.Unlock()
		return s.loose(ctx, v)
	}
	if s.inflight {
		s.mu.Unlock()
		return nil
	}
	v := copyVec(s.peerVec)
	for d := range s.stuck {
		v[d] = math.MaxInt64
	}
	s.mu.Unlock()
	if s.n.isStopped(s.n.self) {
		v[s.n.self] = math.MaxInt64 // this device forked: don't spread its rows
	}
	evs, err := s.n.o.Store.Missing(ctx, v, MaxBatchRows)
	if err != nil || len(evs) == 0 {
		return nil
	}
	f := batches(evs, false)[0]
	ext := map[string][2]int64{}
	for _, r := range f.Events {
		e, ok := ext[r.D]
		if !ok {
			e[0] = r.S
		}
		e[1] = r.S
		ext[r.D] = e
	}
	s.mu.Lock()
	s.inflight, s.sentAt, s.extent = true, s.n.now(), ext
	s.mu.Unlock()
	return []Frame{f}
}

// loose is what goes first on a connection (§5.2): every person.* row the
// peer lacks (so certs arrive before the rows they verify), honoured
// revokes kept in refused_events (they never join the gapless log), and
// fork rows (so every device learns of a fork).
func (s *session) loose(ctx context.Context, peer store.Vector) []Frame {
	st := s.n.o.Store
	var out []store.Event
	f := store.Filter{Kinds: []string{"person."}, Limit: store.MaxListLimit}
	for {
		evs, err := st.List(ctx, f)
		if err != nil || len(evs) == 0 {
			break
		}
		for _, ev := range evs {
			if ev.Seq > peer[ev.DeviceID] {
				out = append(out, ev)
			}
		}
		f.After = evs[len(evs)-1].Cursor()
	}
	if ref, err := st.Refused(ctx, store.MaxListLimit); err == nil {
		for _, ev := range ref {
			if ev.Kind == store.KindPersonDeviceRevoked && len(ev.Sig) > 0 {
				out = append(out, ev)
			}
		}
	}
	if forks, err := st.Forks(ctx); err == nil {
		for _, fk := range forks {
			if fk.Reason == store.ForkTwoRows && len(fk.Other.Sig) > 0 {
				out = append(out, fk.Other)
			}
		}
	}
	return batches(out, true)
}

func (s *session) read(ctx context.Context) {
	for {
		b, err := s.conn.Recv(ctx)
		if err != nil {
			return
		}
		f, err := DecodeFrame(b)
		if err != nil {
			s.conn.Close("protocol")
			return
		}
		if f.Type != FrameHello && !s.helloDone() {
			s.conn.Close("protocol: frame before hello")
			return
		}
		switch f.Type {
		case FrameHello:
			if !s.onHello(ctx, f) {
				return
			}
		case FrameRows:
			s.onRows(ctx, f)
		case FrameVector:
			s.onVector(f)
		case FramePing:
			s.n.alive(s.conn.ID())
			s.control(Frame{Type: FramePong, T: f.T})
		case FramePong:
			s.n.alive(s.conn.ID())
		case FramePersonSecret:
			if s.n.certified(s.conn.ID()) {
				s.n.takeSecret(f)
			}
		case FrameRowAt:
			// Only the asker's own rows: it is checking its own history.
			if f.DeviceID == s.conn.ID() {
				if ev, ok := s.n.rowAt(ctx, f.DeviceID, f.Seq); ok {
					s.control(Frame{Type: FrameRows, Events: []Row{RowOf(ev)}, Loose: true})
				}
			}
		default:
			// A newer peer's frame: ignored ("payloads only gain fields").
		}
	}
}

// onHello checks the peer (§5.2) and records its vector and epoch.
func (s *session) onHello(ctx context.Context, f Frame) bool {
	n := s.n
	peer := s.conn.ID()
	if s.helloDone() {
		return true
	}
	switch {
	case f.V != Protocol:
		s.conn.Close("version")
		return false
	case f.DeviceID != peer || f.PersonID != n.o.Ident.Current().ID:
		s.conn.Close("person")
		return false
	case f.Schema < MinPeerSchema:
		n.mu.Lock()
		n.info(peer).schemaOld = true
		n.mu.Unlock()
		s.conn.Close("version")
		return false
	}
	if !n.o.TrustAnyPeer {
		cert, ok := n.o.Chain.Cert(peer)
		if !ok || hex.EncodeToString(s.conn.Pub()) != cert.DevicePub {
			s.conn.Close("key")
			return false
		}
	}
	now := n.now()
	n.mu.Lock()
	k := n.info(peer)
	moved := k.epoch != f.Epoch || k.pub != f.PersonPub
	k.epoch, k.pub, k.lastSync, k.lastPing = f.Epoch, f.PersonPub, now, now
	k.skew = time.UnixMilli(f.WallMs).Sub(now)
	k.schemaOld = false
	n.mu.Unlock()
	if moved {
		n.savePeers()
	}
	mine, err := n.o.Store.Vector(ctx)
	if err != nil {
		mine = store.Vector{}
	}
	// The peer's copy of my latest row it holds must be the row I hold at
	// that seq. If not, this data dir was restored and wrote over rows it
	// had already sent: a fork. My row goes to the peer, whose Insert
	// records it and stops my stream there too (§5.2).
	at, have := f.Vector[n.self], mine[n.self]
	if f.Top != "" && at > 0 && at <= have {
		if ev, ok := n.rowAt(ctx, n.self, at); ok && store.RowHash(ev) != f.Top {
			n.stop(n.self, forkedSelf)
			s.control(Frame{Type: FrameRows, Events: []Row{RowOf(ev)}, Loose: true})
		}
	}
	// My own rows coming back (§5.2): the peer holds more of them than I
	// do, so this data dir was restored. Appends wait until they are back.
	// Only a claim the peer proves with my own signed row holds them: a
	// made-up vector could never be met, and would block every Append.
	switch {
	case at <= have || n.isStopped(n.self):
	case !n.provesOwn(f.TopRow, at, f.Top):
		n.mu.Lock()
		n.info(peer).refused++
		n.mu.Unlock()
		n.logf("mesh: %s claims %d of this device's rows without the row to prove it; ignored", peer, at)
	default:
		if have > 0 {
			// The rows after mine are only mine if the peer's copy of my
			// last row is the one I hold. If this device wrote other rows
			// since its backup, that's a fork, found before the old rows
			// are stacked on the new ones.
			s.mu.Lock()
			s.ownCheck = have
			s.mu.Unlock()
			s.control(Frame{Type: FrameRowAt, DeviceID: n.self, Seq: have})
		}
		n.o.Store.HoldAppends(at)
		n.mu.Lock()
		if at > n.restoreTo {
			n.restoring, n.restoreTo = peer, at
		}
		n.mu.Unlock()
		n.logf("mesh: restoring this device's rows up to %d from %s", at, peer)
	}
	s.mu.Lock()
	s.helloRecv, s.pEpoch, s.pPub, s.peerVec = true, f.Epoch, f.PersonPub, copyVec(f.Vector)
	s.mu.Unlock()
	n.noteVector(peer, f.Vector)
	n.connected(peer)
	n.sendSecret(s) // if it is behind, or rotated apart from this device
	n.Kick()        // its epoch may close or open a topic
	s.poke()
	return true
}

func (s *session) onVector(f Frame) {
	s.mu.Lock()
	old := s.peerVec
	merged := copyVec(old)
	for d, v := range f.Vector {
		if v > merged[d] {
			merged[d] = v
		}
	}
	for d := range s.stuck {
		if merged[d] > old[d] {
			delete(s.stuck, d)
		}
	}
	if f.Ack && s.inflight {
		for d, ex := range s.extent {
			got := f.Vector[d]
			if got < ex[1] && got >= ex[0]-1 {
				// The peer refused or held part of it: skip that device
				// until it moves or the next tick. Below ex[0]-1 is a gap:
				// resend from its vector.
				s.stuck[d] = true
			}
		}
		s.inflight, s.extent = false, nil
	}
	s.peerVec = merged
	s.mu.Unlock()
	s.n.noteVector(s.conn.ID(), merged)
	s.poke()
}

// onRows applies a rows frame, then acks with this device's vector.
func (s *session) onRows(ctx context.Context, f Frame) {
	if f.Loose {
		// Not an ack: the sender's batch in flight is a different frame.
		s.onLoose(ctx, f.Events)
		s.vector(ctx, false)
		return
	}
	var order []string
	runs := map[string][]store.Event{}
	for _, r := range f.Events {
		if _, ok := runs[r.D]; !ok {
			order = append(order, r.D)
		}
		runs[r.D] = append(runs[r.D], r.Event())
	}
	for _, dev := range order {
		evs := runs[dev]
		if s.n.isStopped(dev) {
			continue
		}
		if s.hasHeld(dev) || (dev == s.n.self && s.checkingOwn()) {
			s.hold(dev, evs)
			continue
		}
		applied, err := s.n.o.Store.InsertBatch(ctx, evs)
		if err != nil {
			s.refusal(dev, evs[applied:], err)
		}
	}
	s.n.synced(s.conn.ID())
	s.ack(ctx)
}

func (s *session) ack(ctx context.Context) { s.vector(ctx, true) }

// vector queues this device's vector; ack marks it as the answer to a rows
// frame, which the sender's next batch waits for.
func (s *session) vector(ctx context.Context, ack bool) {
	v, err := s.n.o.Store.Vector(ctx)
	if err != nil {
		return
	}
	s.control(Frame{Type: FrameVector, Vector: v, Ack: ack})
}

// refusal handles the rest of a device's run after Insert refused rest[0].
func (s *session) refusal(dev string, rest []store.Event, err error) {
	n := s.n
	switch {
	case errors.Is(err, store.ErrNoCert):
		s.hold(dev, rest) // not a forgery: retried when a cert arrives
	case errors.Is(err, store.ErrAhead):
		n.mu.Lock()
		n.ahead[dev] = n.now()
		n.mu.Unlock()
	case dev == n.self && (errors.Is(err, store.ErrFork) || errors.Is(err, store.ErrHLCOrder)):
		// My own rows from before a restore don't follow the ones I wrote
		// since: I signed two histories. My copy goes to the peer too, so
		// it records the fork and stops my stream there (§5.2).
		n.stop(n.self, forkedSelf)
		ctx := context.Background()
		ev, ok := n.rowAt(ctx, n.self, rest[0].Seq)
		if !ok {
			ev, ok = n.rowAt(ctx, n.self, rest[0].Seq-1) // an HLC refusal: the row before differs
		}
		if ok {
			s.control(Frame{Type: FrameRows, Events: []Row{RowOf(ev)}, Loose: true})
		}
	case errors.Is(err, store.ErrFork):
		n.stop(dev, "signed two different histories")
	case errors.Is(err, store.ErrHLCOrder):
		n.stop(dev, "its clock went backwards")
	case errors.Is(err, store.ErrGap), errors.Is(err, store.ErrClosed), errors.Is(err, context.Canceled):
		// The ack vector restarts the sender; a closing store says nothing.
	default:
		n.mu.Lock()
		n.info(s.conn.ID()).refused++
		n.mu.Unlock()
		n.logf("mesh: refused %s/%d from %s: %v", dev, rest[0].Seq, s.conn.ID(), err)
	}
}

// onLoose inserts each loose row on its own. A person row whose device's
// earlier rows aren't held yet is learned instead (Chain.Learn), so the
// rows it certifies verify; learning repeats until nothing more verifies.
func (s *session) onLoose(ctx context.Context, evs []Row) {
	n := s.n
	var learn []store.Event
	for _, r := range evs {
		ev := r.Event()
		if n.isStopped(ev.DeviceID) {
			continue
		}
		err := n.o.Store.Insert(ctx, ev)
		if ev.DeviceID == n.self && err == nil && s.passOwnCheck(ev.Seq) {
			// The peer's copy of my last row is mine: the rows after it
			// are my own history, held until now.
			go s.retryHeld()
		}
		switch {
		case err == nil:
		case store.IsPersonKind(ev.Kind) && (errors.Is(err, store.ErrGap) || errors.Is(err, store.ErrNoCert)):
			learn = append(learn, ev)
		case errors.Is(err, store.ErrNoCert), errors.Is(err, store.ErrGap):
			// A fork row for a device not certified here, or ahead of
			// what is held: it comes again.
		default:
			s.refusal(ev.DeviceID, []store.Event{ev}, err)
		}
	}
	learned := false
	for len(learn) > 0 {
		var rest []store.Event
		for _, ev := range learn {
			switch err := n.o.Chain.Learn(ev); {
			case err == nil:
				learned = true
			case errors.Is(err, store.ErrNoCert):
				rest = append(rest, ev)
			default:
				n.mu.Lock()
				n.info(s.conn.ID()).refused++
				n.mu.Unlock()
			}
		}
		if len(rest) == len(learn) {
			break
		}
		learn = rest
	}
	if learned {
		n.Kick()
	}
}

func (s *session) checkingOwn() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.ownCheck > 0
}

// passOwnCheck ends the own-row check if seq is the row it asked for.
func (s *session) passOwnCheck(seq int64) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.ownCheck == 0 || s.ownCheck != seq {
		return false
	}
	s.ownCheck = 0
	return true
}

func (s *session) hasHeld(dev string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return len(s.held[dev]) > 0
}

// hold keeps rows refused only for "no cert yet", up to the per-peer cap.
func (s *session) hold(dev string, evs []store.Event) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.held == nil {
		s.held = map[string]map[int64]store.Event{}
	}
	m := s.held[dev]
	if m == nil {
		m = map[int64]store.Event{}
		s.held[dev] = m
	}
	for _, ev := range evs {
		if _, ok := m[ev.Seq]; ok {
			continue
		}
		size := len(ev.Payload) + len(ev.Sig)
		if s.heldRows >= HeldRowsCap || s.heldBytes+size > HeldBytesCap {
			return // dropped: asked for again by vector later
		}
		m[ev.Seq] = ev
		s.heldRows++
		s.heldBytes += size
	}
}

// retryHeld inserts held rows again, after a cert may have arrived.
func (s *session) retryHeld() {
	ctx := context.Background()
	s.mu.Lock()
	if len(s.held) == 0 {
		s.mu.Unlock()
		return
	}
	held := s.held
	s.held, s.heldRows, s.heldBytes = nil, 0, 0
	if own := held[s.n.self]; own != nil && s.ownCheck > 0 {
		// Still checking: my own rows stay held.
		delete(held, s.n.self)
		s.held = map[string]map[int64]store.Event{s.n.self: own}
		for _, ev := range own {
			s.heldRows++
			s.heldBytes += len(ev.Payload) + len(ev.Sig)
		}
	}
	s.mu.Unlock()
	any := false
	devs := make([]string, 0, len(held))
	for d := range held {
		devs = append(devs, d)
	}
	sort.Strings(devs)
	for _, dev := range devs {
		m := held[dev]
		evs := make([]store.Event, 0, len(m))
		for _, ev := range m {
			evs = append(evs, ev)
		}
		sort.Slice(evs, func(i, j int) bool { return evs[i].Seq < evs[j].Seq })
		if s.n.isStopped(dev) {
			continue
		}
		applied, err := s.n.o.Store.InsertBatch(ctx, evs)
		if applied > 0 {
			any = true
		}
		if err != nil {
			s.refusal(dev, evs[applied:], err)
		}
	}
	if any {
		s.vector(ctx, false)
	}
}

func copyVec(v store.Vector) store.Vector {
	out := make(store.Vector, len(v))
	for k, x := range v {
		out[k] = x
	}
	return out
}

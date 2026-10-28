package mesh

import (
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"sync"
	"time"
)

// FakeNet is an in-memory Transport for tests: rooms are topics in a map,
// envelopes go to every other member in order, and a connection is a pair
// of Go queues. It runs each side's Allow and Accept with the other's real
// key, as go-room's handshake would, and it can partition and drop links.
// Nothing here opens a socket.
type FakeNet struct {
	mu     sync.Mutex
	rooms  map[string]map[string]*fakeRoom // topic -> self -> room
	cut    map[[2]string]bool              // partitioned device pairs
	conns  map[[2]string][]*fakeConn       // device pair -> both ends of live links
	joins  int
	frames int
}

// NewFakeNet returns an empty network.
func NewFakeNet() *FakeNet {
	return &FakeNet{rooms: map[string]map[string]*fakeRoom{}, cut: map[[2]string]bool{}, conns: map[[2]string][]*fakeConn{}}
}

func pairKey(a, b string) [2]string {
	if a > b {
		a, b = b, a
	}
	return [2]string{a, b}
}

// Join implements Transport.
func (n *FakeNet) Join(s Secret, cfg RoomConfig) (Room, error) {
	if s.Topic == "" || cfg.Self == "" {
		return nil, errors.New("mesh: fake join needs a topic and a self")
	}
	r := &fakeRoom{net: n, topic: s.Topic, cfg: cfg, wake: make(chan struct{}, 1), done: make(chan struct{})}
	n.mu.Lock()
	if n.rooms[s.Topic] == nil {
		n.rooms[s.Topic] = map[string]*fakeRoom{}
	}
	if old := n.rooms[s.Topic][cfg.Self]; old != nil {
		n.mu.Unlock()
		return nil, fmt.Errorf("mesh: %s already joined %s", cfg.Self, s.Topic)
	}
	n.rooms[s.Topic][cfg.Self] = r
	n.joins++
	n.mu.Unlock()
	go r.loop()
	if cfg.OnConnect != nil {
		go cfg.OnConnect()
	}
	return r, nil
}

// Joins counts every Join so far (a single device must make none).
func (n *FakeNet) Joins() int {
	n.mu.Lock()
	defer n.mu.Unlock()
	return n.joins
}

// Members lists who is in topic right now.
func (n *FakeNet) Members(topic string) []string {
	n.mu.Lock()
	defer n.mu.Unlock()
	var out []string
	for id := range n.rooms[topic] {
		out = append(out, id)
	}
	return out
}

// Topics lists the topics self is in.
func (n *FakeNet) Topics(self string) []string {
	n.mu.Lock()
	defer n.mu.Unlock()
	var out []string
	for t, m := range n.rooms {
		if m[self] != nil {
			out = append(out, t)
		}
	}
	return out
}

// Partition cuts every link between a and b: open connections drop and
// envelopes between them are lost until Heal.
func (n *FakeNet) Partition(a, b string) {
	n.mu.Lock()
	n.cut[pairKey(a, b)] = true
	conns := n.conns[pairKey(a, b)]
	delete(n.conns, pairKey(a, b))
	n.mu.Unlock()
	for _, c := range conns {
		c.down(errors.New("mesh: link cut"))
	}
}

// Isolate partitions a from every other member of any room.
func (n *FakeNet) Isolate(a string) {
	for _, b := range n.everyone() {
		if b != a {
			n.Partition(a, b)
		}
	}
}

// Heal undoes Partition.
func (n *FakeNet) Heal(a, b string) {
	n.mu.Lock()
	delete(n.cut, pairKey(a, b))
	n.mu.Unlock()
}

// HealAll undoes every partition.
func (n *FakeNet) HealAll() {
	n.mu.Lock()
	n.cut = map[[2]string]bool{}
	n.mu.Unlock()
}

// Drop closes the open connections between a and b once, as a failed path
// would; the link stays, so they reconnect.
func (n *FakeNet) Drop(a, b string) {
	n.mu.Lock()
	conns := n.conns[pairKey(a, b)]
	delete(n.conns, pairKey(a, b))
	n.mu.Unlock()
	for _, c := range conns {
		c.down(errors.New("mesh: connection dropped"))
	}
}

// Connected reports whether a and b have a live connection.
func (n *FakeNet) Connected(a, b string) bool {
	n.mu.Lock()
	defer n.mu.Unlock()
	for _, c := range n.conns[pairKey(a, b)] {
		select {
		case <-c.done:
		default:
			return true
		}
	}
	return false
}

// Frames counts every sync message sent.
func (n *FakeNet) Frames() int {
	n.mu.Lock()
	defer n.mu.Unlock()
	return n.frames
}

func (n *FakeNet) everyone() []string {
	n.mu.Lock()
	defer n.mu.Unlock()
	seen := map[string]bool{}
	var out []string
	for _, m := range n.rooms {
		for id := range m {
			if !seen[id] {
				seen[id] = true
				out = append(out, id)
			}
		}
	}
	return out
}

type fakeRoom struct {
	net   *FakeNet
	topic string
	cfg   RoomConfig

	mu     sync.Mutex
	queue  []Envelope
	wake   chan struct{}
	done   chan struct{}
	closed bool
}

func (r *fakeRoom) Send(to, typ string, body any) error {
	raw, err := json.Marshal(body)
	if err != nil {
		return err
	}
	env := Envelope{From: r.cfg.Self, Type: typ, Body: raw}
	n := r.net
	n.mu.Lock()
	if r.isClosed() {
		n.mu.Unlock()
		return errors.New("mesh: room closed")
	}
	var dst []*fakeRoom
	for id, m := range n.rooms[r.topic] {
		if id == r.cfg.Self || (to != "" && to != id) || n.cut[pairKey(id, r.cfg.Self)] {
			continue
		}
		dst = append(dst, m)
	}
	n.mu.Unlock()
	for _, m := range dst {
		m.enqueue(env)
	}
	return nil
}

func (r *fakeRoom) enqueue(env Envelope) {
	r.mu.Lock()
	r.queue = append(r.queue, env)
	r.mu.Unlock()
	select {
	case r.wake <- struct{}{}:
	default:
	}
}

func (r *fakeRoom) loop() {
	for {
		select {
		case <-r.done:
			return
		case <-r.wake:
		}
		for {
			r.mu.Lock()
			if len(r.queue) == 0 {
				r.mu.Unlock()
				break
			}
			env := r.queue[0]
			r.queue = r.queue[1:]
			r.mu.Unlock()
			if r.cfg.OnMsg != nil {
				r.cfg.OnMsg(env)
			}
		}
	}
}

func (r *fakeRoom) isClosed() bool {
	select {
	case <-r.done:
		return true
	default:
		return false
	}
}

// Connect links r to peer's room on the same topic when both sides allow
// and accept each other, as go-room's offer, answer and handshake would.
func (r *fakeRoom) Connect(peer string) {
	n := r.net
	n.mu.Lock()
	other := n.rooms[r.topic][peer]
	k := pairKey(r.cfg.Self, peer)
	blocked := other == nil || peer == r.cfg.Self || n.cut[k] || r.isClosed()
	if !blocked {
		for _, c := range n.conns[k] {
			if c.topic == r.topic && !c.isDown() {
				blocked = true // already up; Connect is idempotent
				break
			}
		}
	}
	n.mu.Unlock()
	if blocked {
		return
	}
	if r.cfg.Allow != nil && !r.cfg.Allow(peer) {
		return
	}
	if other.cfg.Allow != nil && !other.cfg.Allow(r.cfg.Self) {
		return
	}
	if r.cfg.Accept(peer, other.cfg.Pub) != nil || other.cfg.Accept(r.cfg.Self, r.cfg.Pub) != nil {
		return
	}
	a, b := newFakePair(n, r, other)
	n.mu.Lock()
	// Re-check under the lock: the other side may have connected meanwhile.
	for _, c := range n.conns[k] {
		if c.topic == r.topic && !c.isDown() {
			n.mu.Unlock()
			return
		}
	}
	if n.cut[k] || r.isClosed() || other.isClosed() {
		n.mu.Unlock()
		return
	}
	n.conns[k] = append(n.conns[k], a, b)
	n.mu.Unlock()
	if r.cfg.OnPeer != nil {
		go r.cfg.OnPeer(a)
	}
	if other.cfg.OnPeer != nil {
		go other.cfg.OnPeer(b)
	}
}

func (r *fakeRoom) Close() {
	n := r.net
	n.mu.Lock()
	if r.closed {
		n.mu.Unlock()
		return
	}
	r.closed = true
	close(r.done)
	if n.rooms[r.topic][r.cfg.Self] == r {
		delete(n.rooms[r.topic], r.cfg.Self)
	}
	var mine []*fakeConn
	for k, list := range n.conns {
		keep := list[:0]
		for _, c := range list {
			if c.topic == r.topic && (c.self == r.cfg.Self || c.peer == r.cfg.Self) {
				mine = append(mine, c)
				continue
			}
			keep = append(keep, c)
		}
		n.conns[k] = keep
	}
	n.mu.Unlock()
	for _, c := range mine {
		c.down(errors.New("mesh: room closed"))
	}
}

// fakeConn is one end of a fake connection.
type fakeConn struct {
	net         *FakeNet
	topic       string
	self, peer  string
	pub         ed25519.PublicKey
	session     string
	other       *fakeConn
	in          chan []byte
	done        chan struct{}
	once        sync.Once
	mu          sync.Mutex
	err         error
	onStream    func(string, io.ReadWriteCloser)
	pendStreams []fakeStream
}

type fakeStream struct {
	label string
	c     net.Conn
}

func newFakePair(n *FakeNet, a, b *fakeRoom) (*fakeConn, *fakeConn) {
	sum := sha256.Sum256([]byte(fmt.Sprintf("%s|%s|%s|%d", a.topic, a.cfg.Self, b.cfg.Self, time.Now().UnixNano())))
	sess := hex.EncodeToString(sum[:8])
	x := &fakeConn{net: n, topic: a.topic, self: a.cfg.Self, peer: b.cfg.Self, pub: b.cfg.Pub, session: sess,
		in: make(chan []byte, 1024), done: make(chan struct{})}
	y := &fakeConn{net: n, topic: a.topic, self: b.cfg.Self, peer: a.cfg.Self, pub: a.cfg.Pub, session: sess,
		in: make(chan []byte, 1024), done: make(chan struct{})}
	x.other, y.other = y, x
	return x, y
}

func (c *fakeConn) ID() string             { return c.peer }
func (c *fakeConn) Pub() ed25519.PublicKey { return c.pub }
func (c *fakeConn) Done() <-chan struct{}  { return c.done }
func (c *fakeConn) Path() string           { return "direct" }
func (c *fakeConn) RTT() time.Duration     { return time.Millisecond }

func (c *fakeConn) Fingerprints() (string, string) {
	return "fake:" + c.session + ":" + c.self, "fake:" + c.session + ":" + c.peer
}

func (c *fakeConn) isDown() bool {
	select {
	case <-c.done:
		return true
	default:
		return false
	}
}

func (c *fakeConn) Err() error {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.err
}

func (c *fakeConn) Send(ctx context.Context, msg []byte) error {
	if c.isDown() {
		return c.Err()
	}
	c.net.mu.Lock()
	c.net.frames++
	c.net.mu.Unlock()
	cp := append([]byte(nil), msg...)
	select {
	case c.other.in <- cp:
		return nil
	case <-c.done:
		return c.Err()
	case <-c.other.done:
		return c.other.Err()
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (c *fakeConn) Recv(ctx context.Context) ([]byte, error) {
	select {
	case m := <-c.in:
		return m, nil
	default:
	}
	select {
	case m := <-c.in:
		return m, nil
	case <-c.done:
		return nil, c.Err()
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

func (c *fakeConn) OpenStream(ctx context.Context, label string) (io.ReadWriteCloser, error) {
	if c.isDown() {
		return nil, c.Err()
	}
	mine, theirs := net.Pipe() // in memory: no listener, no socket
	o := c.other
	o.mu.Lock()
	h := o.onStream
	if h == nil {
		o.pendStreams = append(o.pendStreams, fakeStream{label, theirs})
	}
	o.mu.Unlock()
	if h != nil {
		go h(label, theirs)
	}
	go func() {
		select {
		case <-c.done:
		case <-o.done:
		}
		mine.Close()
		theirs.Close()
	}()
	return mine, nil
}

func (c *fakeConn) OnStream(fn func(string, io.ReadWriteCloser)) {
	c.mu.Lock()
	c.onStream = fn
	pend := c.pendStreams
	c.pendStreams = nil
	c.mu.Unlock()
	for _, s := range pend {
		go fn(s.label, s.c)
	}
}

func (c *fakeConn) Close(reason string) {
	c.down(fmt.Errorf("mesh: closed: %s", reason))
}

// down takes both ends down: this one with err, the other as a bye.
func (c *fakeConn) down(err error) {
	c.downOne(err)
	c.other.downOne(fmt.Errorf("mesh: peer went away: %w", err))
}

func (c *fakeConn) downOne(err error) {
	c.once.Do(func() {
		c.mu.Lock()
		c.err = err
		c.mu.Unlock()
		close(c.done)
	})
}

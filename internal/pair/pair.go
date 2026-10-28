package pair

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/store"
)

// Window is how long a pairing stays open (§3 step 1): one use, 10 minutes.
const Window = 10 * time.Minute

// HelloEvery is how often B repeats pair.hello until A connects: envelopes
// aren't kept by the brokers, so a hello sent before A joined is lost.
const HelloEvery = 2 * time.Second

// MaxMisses is how many wrong typings close the window (§3 step 5).
const MaxMisses = 2

// Envelope types in the pairing room (§4.2).
const (
	MsgHello = "pair.hello"
	MsgBusy  = "pair.busy"
)

// Frames on the pairing connection.
const (
	frameProof  = "pair.proof"
	frameGrant  = "pair.grant"
	frameDone   = "pair.done"
	frameFail   = "pair.fail"
	frameCancel = "pair.cancel"
)

// Modes and states (Status).
const (
	ModeAdd  = "add"
	ModeJoin = "join"

	StateWaiting    = "waiting"   // add: the code is shown; join: looking for the other device
	StateConfirm    = "confirm"   // add: type the words B shows
	StateWords      = "words"     // join: read these words out
	StateDone       = "done"      // paired
	StateFailed     = "failed"    // the window closed (Error says why)
	StateCanceled   = "canceled"  // the person canceled
	StateExpired    = "expired"   // nobody finished within Window
	StateCodeInUse  = "code_used" // join: A was already pairing with someone (pair.busy)
	StateNotCreated = ""          // nothing has been opened
)

// The person-facing failure texts (§3 step 5).
const (
	ErrTextSomeoneElse = "Someone else tried to use this code. Make a new one."
	ErrTextCodeUsed    = "That code was already used. Make a new one on the other device."
)

// Errors.
var (
	ErrBusy         = errors.New("pair: a pairing is already open; finish or cancel it first")
	ErrInMesh       = errors.New("pair: this device is already one of a person's devices; remove it there or reset it first")
	ErrNotOpen      = errors.New("pair: no pairing is waiting for words")
	ErrWrongWords   = errors.New("pair: those aren't the words the other device shows")
	ErrNotCertified = errors.New("pair: this device isn't certified, so it can't add another")
)

// Joiner is the device on the other end, as the add side shows it above
// the words box (§3 step 4).
type Joiner struct {
	DeviceID    string `json:"deviceId"`
	Name        string `json:"name"`
	OS          string `json:"os"`
	Fingerprint string `json:"fingerprint,omitempty"`
}

// Status is the current (or last) pairing, for the Devices dialog.
type Status struct {
	Mode    string    `json:"mode"`
	State   string    `json:"state"`
	Code    string    `json:"code,omitempty"` // add, while waiting
	QR      string    `json:"qr,omitempty"`   // add: the QR code's text
	Expires time.Time `json:"expires,omitzero"`
	Role    string    `json:"role,omitempty"` // add: the role B gets
	// Joiner is B (add side) once it connected and proved the code.
	Joiner *Joiner `json:"joiner,omitempty"`
	// Words are what B shows (join side) once both proved the code.
	Words []string `json:"words,omitempty"`
	// Peer is A's name (join side).
	Peer   string `json:"peer,omitempty"`
	Misses int    `json:"misses,omitempty"`
	Error  string `json:"error,omitempty"`
}

// Open reports whether the pairing is still in progress.
func (s Status) Open() bool {
	switch s.State {
	case StateWaiting, StateConfirm, StateWords:
		return true
	}
	return false
}

// Options configure a Pairer.
type Options struct {
	Net   mesh.Transport
	Ident *identity.Identity
	Chain *identity.Chain
	Store *store.Store
	// DataDir is where B keeps the person rows it was handed until they
	// are stored (SavedRows).
	DataDir string
	// Kick re-reads the cert fold in the net layer (mesh.Node.Kick), so it
	// starts once the person has a second device.
	Kick func()
	// Window and HelloEvery shorten the timers (tests).
	Window     time.Duration
	HelloEvery time.Duration
	Now        func() time.Time
	Logf       func(format string, args ...any)
	// OnChange is told whenever Status changes (the GUI refreshes).
	OnChange func(Status)
}

// Pairer runs at most one pairing at a time, either side.
type Pairer struct {
	o Options

	mu  sync.Mutex
	cur *session
	st  Status
}

// New returns a Pairer.
func New(o Options) *Pairer {
	if o.Window <= 0 {
		o.Window = Window
	}
	if o.HelloEvery <= 0 {
		o.HelloEvery = HelloEvery
	}
	if o.Now == nil {
		o.Now = time.Now
	}
	if o.Logf == nil {
		o.Logf = func(string, ...any) {}
	}
	if o.Kick == nil {
		o.Kick = func() {}
	}
	return &Pairer{o: o}
}

// Status is the current or last pairing.
func (p *Pairer) Status() Status {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.copyStatus()
}

func (p *Pairer) copyStatus() Status {
	st := p.st
	if st.Joiner != nil {
		j := *st.Joiner
		st.Joiner = &j
	}
	st.Words = append([]string(nil), st.Words...)
	return st
}

// SetOnChange replaces Options.OnChange (the GUI subscribes after the Core
// is up).
func (p *Pairer) SetOnChange(fn func(Status)) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.o.OnChange = fn
}

// changed publishes Status; p.mu is held.
func (p *Pairer) changed() {
	if p.o.OnChange != nil {
		st := p.copyStatus()
		go p.o.OnChange(st)
	}
}

// session is one open pairing window.
type session struct {
	p      *Pairer
	add    bool
	secret mesh.Secret
	role   string
	room   mesh.Room
	ctx    context.Context
	cancel context.CancelFunc
	timer  *time.Timer

	// guarded by p.mu
	joiner  string // add: the first hello's device id
	hello   helloBody
	pinID   string // the one peer this side lets up, and its key
	pinPub  ed25519.PublicKey
	conn    mesh.Conn
	words   []string
	misses  int
	sent    bool // add: the grant went out; a closing connection now means done
	closed  bool
	granted chan struct{} // add: closed once pair.done arrives
}

type helloBody struct {
	DeviceID string `json:"deviceId"`
	Name     string `json:"name"`
	OS       string `json:"os"`
}

// frame is one message on the pairing connection. Nothing in it is logged.
type frame struct {
	Type   string           `json:"type"`
	Proof  []byte           `json:"proof,omitempty"`
	Name   string           `json:"name,omitempty"`
	OS     string           `json:"os,omitempty"`
	Person *identity.Person `json:"person,omitempty"`
	Seed   []byte           `json:"seed,omitempty"`
	Rows   []mesh.Row       `json:"rows,omitempty"`
	Reason string           `json:"reason,omitempty"`
}

// self is this device's id.
func (p *Pairer) self() string { return p.o.Ident.Device.ID }

// InMesh reports whether this device already belongs to a mesh of more than
// itself (§3): it joined another device's person, or its person has another
// certified device.
func (p *Pairer) InMesh() bool {
	if p.o.Ident.Current().RootDeviceID != p.self() {
		return true
	}
	for _, d := range p.o.Chain.Devices() {
		if d.Cert.DeviceID != p.self() {
			return true
		}
	}
	return false
}

// Add opens a pairing window on this device (A, §3 step 1): a fresh code,
// its QR text, and the pairing room. role is what the joiner's cert says
// (identity.RoleDesktop or RoleNode).
func (p *Pairer) Add(role string) (Status, error) {
	if role != identity.RoleNode {
		role = identity.RoleDesktop
	}
	if _, ok := p.o.Chain.Cert(p.self()); !ok {
		return Status{}, ErrNotCertified
	}
	if _, revoked := p.o.Chain.Cut(p.self()); revoked {
		return Status{}, ErrNotCertified
	}
	code := NewCode()
	secret, err := RoomSecret(code)
	if err != nil {
		return Status{}, err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.cur != nil {
		return Status{}, ErrBusy
	}
	s := p.newSession(true, secret, role)
	if err := s.join(); err != nil {
		s.cancel()
		return Status{}, fmt.Errorf("pair: open the pairing room: %w", err)
	}
	p.cur = s
	p.st = Status{Mode: ModeAdd, State: StateWaiting, Code: code, QR: QR(code), Expires: p.o.Now().Add(p.o.Window), Role: role}
	s.timer = time.AfterFunc(p.o.Window, func() { s.fail(StateExpired, "Nobody used the code in time. Make a new one.", "") })
	p.changed()
	return p.copyStatus(), nil
}

// Join enters a code on this device (B, §3 step 2). It returns at once;
// Status moves to StateWords when the words are ready to read out.
func (p *Pairer) Join(code string) (Status, error) {
	canon, ok := NormalizeCode(code)
	if !ok {
		return Status{}, ErrBadCode
	}
	if p.InMesh() {
		return Status{}, ErrInMesh
	}
	secret, err := RoomSecret(canon)
	if err != nil {
		return Status{}, err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.cur != nil {
		return Status{}, ErrBusy
	}
	s := p.newSession(false, secret, "")
	if err := s.join(); err != nil {
		s.cancel()
		return Status{}, fmt.Errorf("pair: open the pairing room: %w", err)
	}
	p.cur = s
	p.st = Status{Mode: ModeJoin, State: StateWaiting, Expires: p.o.Now().Add(p.o.Window)}
	s.timer = time.AfterFunc(p.o.Window, func() { s.fail(StateExpired, "The other device didn't answer in time.", "") })
	go s.helloLoop()
	p.changed()
	return p.copyStatus(), nil
}

// Cancel closes the open pairing, if any (B's Cancel, or A giving up).
func (p *Pairer) Cancel() {
	p.mu.Lock()
	s := p.cur
	p.mu.Unlock()
	if s != nil {
		s.fail(StateCanceled, "Canceled.", frameCancel)
	}
}

// Confirm is the person typing B's words into A (§3 step 4). A match
// certifies B and hands it the person; a miss counts, and the second one
// closes the window.
func (p *Pairer) Confirm(ctx context.Context, typed string) error {
	p.mu.Lock()
	s := p.cur
	if s == nil || !s.add || p.st.State != StateConfirm {
		p.mu.Unlock()
		return ErrNotOpen
	}
	if !WordsMatch(s.words, typed) {
		s.misses++
		p.st.Misses = s.misses
		miss := s.misses
		p.changed()
		p.mu.Unlock()
		if miss >= MaxMisses {
			s.fail(StateFailed, ErrTextSomeoneElse, frameFail)
		}
		return ErrWrongWords
	}
	conn, joiner, hello, role := s.conn, s.joiner, s.hello, s.role
	pub := s.pinPub
	p.mu.Unlock()
	if err := s.grant(ctx, conn, joiner, pub, hello, role); err != nil {
		s.fail(StateFailed, "Pairing failed: "+err.Error(), frameFail)
		return err
	}
	return nil
}

func (p *Pairer) newSession(add bool, secret mesh.Secret, role string) *session {
	ctx, cancel := context.WithCancel(context.Background())
	return &session{p: p, add: add, secret: secret, role: role, ctx: ctx, cancel: cancel, granted: make(chan struct{})}
}

// join opens the pairing room. p.mu is held.
func (s *session) join() error {
	p := s.p
	r, err := p.o.Net.Join(s.secret, mesh.RoomConfig{
		Self:   p.self(),
		Pub:    p.o.Ident.DevicePub(),
		Sign:   p.o.Ident.Sign,
		Accept: s.accept,
		Allow:  s.allow,
		// In the pairing room A always offers (§3 step 2).
		Offerer: func(string) bool { return s.add },
		OnMsg:   s.onMsg,
		OnPeer:  s.onPeer,
	})
	if err != nil {
		return err
	}
	s.room = r
	return nil
}

// allow lets up only the one device this side is pairing with: on A the
// first hello's sender, on B the first device that offers.
func (s *session) allow(peer string) bool {
	p := s.p
	p.mu.Lock()
	defer p.mu.Unlock()
	if s.closed || peer == p.self() {
		return false
	}
	if s.add {
		return peer == s.joiner
	}
	return s.pinID == "" || s.pinID == peer
}

// accept takes any key once, as nothing is certified yet, and refuses a
// different one for the same device afterwards, so a code holder can't
// swap in its own key once the connection is up (§3 step 2). A refuses a
// device that is itself, revoked, or already certified.
func (s *session) accept(peer string, pub ed25519.PublicKey) error {
	p := s.p
	if peer == p.self() {
		return mesh.ErrRefused
	}
	if s.add {
		if cert, ok := p.o.Chain.Cert(peer); ok {
			_, revoked := p.o.Chain.Cut(peer)
			if revoked || cert.DevicePub != hex.EncodeToString(pub) {
				return mesh.ErrRefused // a copied data dir or restored backup (§5.2)
			}
			return mesh.ErrRefused // already one of the person's devices
		}
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	switch {
	case s.closed:
		return mesh.ErrRefused
	case s.add && peer != s.joiner:
		return mesh.ErrRefused
	case s.pinID == "":
		s.pinID, s.pinPub = peer, append(ed25519.PublicKey(nil), pub...)
		return nil
	case s.pinID != peer || !bytes.Equal(s.pinPub, pub):
		return mesh.ErrRefused
	}
	return nil
}

func (s *session) onMsg(env mesh.Envelope) {
	p := s.p
	switch env.Type {
	case MsgHello:
		if !s.add {
			return
		}
		var h helloBody
		if json.Unmarshal(env.Body, &h) != nil || h.DeviceID == "" || h.DeviceID != env.From {
			return
		}
		if h.DeviceID == p.self() {
			return
		}
		if _, revoked := p.o.Chain.Cut(h.DeviceID); revoked {
			return // a removed device: re-pairing it is a reset plus a join (§3)
		}
		p.mu.Lock()
		if s.closed {
			p.mu.Unlock()
			return
		}
		switch {
		case s.joiner == "":
			s.joiner, s.hello = h.DeviceID, h
		case s.joiner != h.DeviceID:
			// A second device has the code: it gets pair.busy, and the
			// window closes (§3 steps 4 and 5).
			p.mu.Unlock()
			_ = s.room.Send(h.DeviceID, MsgBusy, struct{}{})
			s.fail(StateFailed, ErrTextSomeoneElse, frameFail)
			return
		case s.conn != nil:
			p.mu.Unlock()
			return // already connected; a repeated hello
		}
		p.mu.Unlock()
		s.room.Connect(h.DeviceID)
	case MsgBusy:
		if !s.add {
			s.fail(StateCodeInUse, ErrTextCodeUsed, "")
		}
	}
}

// helloLoop repeats B's hello until a connection is up or the window ends.
func (s *session) helloLoop() {
	p := s.p
	h := helloBody{DeviceID: p.self(), Name: p.o.Ident.Device.Name, OS: p.o.Ident.Device.OS}
	t := time.NewTicker(p.o.HelloEvery)
	defer t.Stop()
	for {
		p.mu.Lock()
		up := s.conn != nil || s.closed
		p.mu.Unlock()
		if up {
			return
		}
		_ = s.room.Send("", MsgHello, h)
		select {
		case <-s.ctx.Done():
			return
		case <-t.C:
		}
	}
}

func (s *session) onPeer(c mesh.Conn) {
	p := s.p
	p.mu.Lock()
	if s.closed || c.ID() != s.pinID {
		p.mu.Unlock()
		c.Close("not pairing")
		return
	}
	if s.conn != nil {
		// The connection was replaced: the words were (or would be)
		// computed on the old one, so pairing starts over (§3 step 6).
		p.mu.Unlock()
		c.Close("replaced")
		s.fail(StateFailed, "The connection changed while pairing. Make a new code.", frameFail)
		return
	}
	s.conn = c
	p.mu.Unlock()
	go s.run(c)
}

func (s *session) send(ctx context.Context, c mesh.Conn, f frame) error {
	b, err := json.Marshal(f)
	if err != nil {
		return err
	}
	return c.Send(ctx, b)
}

func (s *session) recv(ctx context.Context, c mesh.Conn) (frame, error) {
	b, err := c.Recv(ctx)
	if err != nil {
		return frame{}, err
	}
	var f frame
	if err := json.Unmarshal(b, &f); err != nil || f.Type == "" {
		return frame{}, errors.New("pair: bad frame")
	}
	return f, nil
}

// run is the SAS exchange and what follows it on one side (§3 steps 3-6).
func (s *session) run(c mesh.Conn) {
	p := s.p
	ctx := s.ctx
	selfPub := p.o.Ident.DevicePub()
	local, remote := c.Fingerprints()
	var info []byte
	mySide, theirSide := byte('B'), byte('A')
	if s.add {
		info = Info(selfPub, c.Pub(), local, remote, p.self(), c.ID())
		mySide, theirSide = 'A', 'B'
	} else {
		info = Info(c.Pub(), selfPub, remote, local, c.ID(), p.self())
	}
	go func() {
		select {
		case <-c.Done():
			p.mu.Lock()
			sent := s.sent
			p.mu.Unlock()
			if sent {
				s.finish() // B took the grant and closed
				return
			}
			s.fail(StateFailed, "The other device went away before pairing finished.", "")
		case <-ctx.Done():
		}
	}()
	hello := frame{Type: frameProof, Proof: Proof(s.secret.Key, mySide, info), Name: p.o.Ident.Device.Name, OS: p.o.Ident.Device.OS}
	if err := s.send(ctx, c, hello); err != nil {
		s.fail(StateFailed, "The other device went away before pairing finished.", "")
		return
	}
	f, err := s.recv(ctx, c)
	if err != nil {
		s.fail(StateFailed, "The other device went away before pairing finished.", "")
		return
	}
	if f.Type != frameProof || !CheckProof(s.secret.Key, theirSide, info, f.Proof) {
		if s.add {
			s.fail(StateFailed, ErrTextSomeoneElse, frameFail)
		} else {
			s.fail(StateFailed, "The other device couldn't prove it has this code.", frameFail)
		}
		return
	}
	words := SAS(s.secret.Key, info)
	p.mu.Lock()
	if s.closed {
		p.mu.Unlock()
		return
	}
	s.words = words
	if s.add {
		p.st.State = StateConfirm
		p.st.Code, p.st.QR = "", ""
		j := &Joiner{DeviceID: c.ID(), Name: s.hello.Name, OS: s.hello.OS, Fingerprint: Fingerprint(c.Pub())}
		if f.Name != "" {
			j.Name = f.Name
		}
		if f.OS != "" {
			j.OS = f.OS
		}
		p.st.Joiner = j
	} else {
		p.st.State, p.st.Words, p.st.Peer = StateWords, words, f.Name
	}
	p.changed()
	p.mu.Unlock()

	// After the words, A waits for Confirm (and for B's done once it
	// granted); B waits for the grant. Either side may fail or cancel.
	for {
		f, err := s.recv(ctx, c)
		if err != nil {
			return // the watcher above reports a lost connection
		}
		switch f.Type {
		case frameFail, frameCancel:
			msg := "The other device canceled."
			if f.Type == frameFail {
				msg = "The other device closed the pairing."
			}
			s.fail(StateFailed, msg, "")
			return
		case frameDone:
			if s.add {
				select {
				case <-s.granted:
				default:
					close(s.granted)
				}
			}
		case frameGrant:
			if s.add {
				continue
			}
			if err := s.adopt(ctx, c, f); err != nil {
				p.o.Logf("pair: refusing the grant: %v", err)
				s.fail(StateFailed, "The other device's answer didn't check out: "+err.Error(), frameFail)
				return
			}
			_ = s.send(ctx, c, frame{Type: frameDone})
			s.finish()
			return
		}
	}
}

// grant is A after the words matched (§3 step 6): certify B, then send it
// the person secret, its epoch, the root, and every person row, on the very
// connection the words were computed on.
func (s *session) grant(ctx context.Context, c mesh.Conn, joiner string, pub ed25519.PublicKey, h helloBody, role string) error {
	p := s.p
	if c == nil || joiner == "" || len(pub) == 0 {
		return ErrNotOpen
	}
	select {
	case <-c.Done():
		return errors.New("the connection went down")
	default:
	}
	label := h.Name
	if label == "" {
		label = joiner
	}
	add, err := p.o.Ident.CertFor(joiner, hex.EncodeToString(pub), label, role, p.o.Now())
	if err != nil {
		return err
	}
	if _, err := p.o.Store.Append(ctx, "irori", store.KindPersonDeviceAdded, add); err != nil {
		return err
	}
	rows, err := personRows(ctx, p.o.Store)
	if err != nil {
		return err
	}
	cur := p.o.Ident.Current()
	seed, ok := p.o.Ident.PersonSeed(cur.Epoch)
	if !ok {
		return errors.New("this device doesn't hold its person's secret")
	}
	p.mu.Lock()
	s.sent = true
	p.mu.Unlock()
	if err := s.send(ctx, c, frame{Type: frameGrant, Person: &cur, Seed: seed, Rows: rows}); err != nil {
		p.mu.Lock()
		s.sent = false
		p.mu.Unlock()
		return err
	}
	p.o.Kick()
	// B answers done once it adopted the person; the cert is written either
	// way, so a lost answer only means the pairing connection closes late.
	select {
	case <-s.granted:
	case <-c.Done():
	case <-time.After(10 * time.Second):
	}
	s.finish()
	return nil
}

// personRows is every person.* row held, oldest first.
func personRows(ctx context.Context, st *store.Store) ([]mesh.Row, error) {
	var out []mesh.Row
	f := store.Filter{Kinds: []string{"person."}, Limit: store.MaxListLimit}
	for {
		evs, err := st.List(ctx, f)
		if err != nil {
			return nil, err
		}
		for _, ev := range evs {
			out = append(out, mesh.RowOf(ev))
		}
		if len(evs) < f.Limit {
			return out, nil
		}
		f.After = evs[len(evs)-1].Cursor()
	}
}

// adopt is B taking A's grant (§3 step 6): the rows must certify this
// device under its own key and A under the key its handshake proved, and
// the secret must match the person key, before anything is replaced.
func (s *session) adopt(ctx context.Context, c mesh.Conn, f frame) error {
	p := s.p
	if f.Person == nil || len(f.Seed) == 0 {
		return errors.New("no person in it")
	}
	person := *f.Person
	evs := make([]store.Event, 0, len(f.Rows))
	for _, r := range f.Rows {
		if !store.IsPersonKind(r.K) {
			return fmt.Errorf("a %s row in it", r.K)
		}
		evs = append(evs, r.Event())
	}
	trial := identity.NewChain(person.ID, person.RootDeviceID, p.self())
	learnAll(trial, evs)
	mine, ok := trial.Cert(p.self())
	if !ok || mine.DevicePub != hex.EncodeToString(p.o.Ident.DevicePub()) {
		return errors.New("it doesn't certify this device")
	}
	theirs, ok := trial.Cert(c.ID())
	if !ok || theirs.DevicePub != hex.EncodeToString(c.Pub()) {
		return errors.New("it doesn't certify the device that sent it")
	}
	if p.InMesh() {
		return ErrInMesh
	}
	// Saved first: if this device stops before it syncs, the rows come
	// back at the next start and the net layer still finds the mesh.
	if err := saveRows(p.o.DataDir, f.Rows); err != nil {
		return err
	}
	if err := p.o.Ident.AdoptPerson(person, f.Seed); err != nil {
		return err
	}
	p.o.Chain.Repin(person.ID, person.RootDeviceID)
	p.o.Store.ReloadVerifier(ctx)
	learnAll(p.o.Chain, evs)
	p.o.Kick()
	return nil
}

// learnAll folds rows that aren't stored yet into ch, in as many passes as
// it takes for each cert's writer to be certified first.
func learnAll(ch *identity.Chain, evs []store.Event) {
	left := evs
	for len(left) > 0 {
		var next []store.Event
		for _, ev := range left {
			if ch.Learn(ev) != nil {
				next = append(next, ev)
			}
		}
		if len(next) == len(left) {
			return
		}
		left = next
	}
}

// finish closes the window as paired.
func (s *session) finish() {
	p := s.p
	p.mu.Lock()
	if s.closed {
		p.mu.Unlock()
		return
	}
	s.closed = true
	p.st.State, p.st.Error, p.st.Code, p.st.QR = StateDone, "", "", ""
	p.st.Words = nil
	if p.cur == s {
		p.cur = nil
	}
	p.changed()
	p.mu.Unlock()
	s.close()
}

// fail closes the window with state and why; tell, if set, is the frame
// the other side gets first.
func (s *session) fail(state, why, tell string) {
	p := s.p
	p.mu.Lock()
	if s.closed {
		p.mu.Unlock()
		return
	}
	s.closed = true
	conn := s.conn
	p.st.State, p.st.Error, p.st.Code, p.st.QR = state, why, "", ""
	p.st.Words = nil
	if p.cur == s {
		p.cur = nil
	}
	p.changed()
	p.mu.Unlock()
	if tell != "" && conn != nil {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		_ = s.send(ctx, conn, frame{Type: tell})
		cancel()
	}
	s.close()
}

// close leaves the pairing room and drops its connection.
func (s *session) close() {
	if s.timer != nil {
		s.timer.Stop()
	}
	s.cancel()
	s.p.mu.Lock()
	conn := s.conn
	s.p.mu.Unlock()
	if conn != nil {
		conn.Close("pairing over")
	}
	if s.room != nil {
		s.room.Close()
	}
}

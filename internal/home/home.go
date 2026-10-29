// Package home is the home server (design/v0.4-devices.md §7.1, decision
// 33): the always-on PC is Irori's server for the person's devices. It
// hosts the signaling hub that carries the sealed envelopes, a TURN relay
// for networks that block direct paths, and the router port mapping that
// makes both reachable from outside the home, and it publishes where it can
// be reached as a node.addr row. No third party is involved.
//
// It runs only on a device where it is switched on, and only once that
// device has another device or an open pairing window (§4.5): a server on
// its own binds nothing.
package home

import (
	"context"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"math/rand/v2"
	"net"
	"net/netip"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"sync"
	"time"

	room "github.com/StephenSHorton/go-room"
	"github.com/StephenSHorton/go-room/portmap"
	"github.com/jackpal/gateway"

	"github.com/StephenSHorton/irori/internal/awake"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/mesh/roomnet"
	"github.com/StephenSHorton/irori/internal/store"
)

// File is the home server's device-local settings (§5.1).
const File = "home-server.json"

// KeyFile is the TURN relay's HMAC key: made on the server, sealed like the
// device key, and never sent anywhere (§4.4).
const KeyFile = "turn.key"

// The port N is picked from this range on first start (§7.1): below the
// ephemeral ranges of Windows and macOS, and not one fixed number, so a
// scan for Irori's default port finds nothing.
const (
	portMin = 20000
	portMax = 29999
)

// Realm is the TURN realm: nothing in it names Irori or the person.
const Realm = "relay"

// Settings is home-server.json.
type Settings struct {
	On   bool `json:"on"`
	Port int  `json:"port,omitempty"`
	// Outside is "Reachable from outside home" (port mapping); nil is the
	// default, on.
	Outside *bool `json:"outside,omitempty"`
	// Public is an address the person entered (an IP or a host name they
	// already have, optionally with a port) for when mapping fails.
	Public string `json:"public,omitempty"`
	// TCPExternal and UDPExternal are the router ports last granted, asked
	// for again so a mapping that moved keeps its number.
	TCPExternal uint16 `json:"tcpExternal,omitempty"`
	UDPExternal uint16 `json:"udpExternal,omitempty"`
	// Awake is "Keep this PC awake while it's the server"; nil is the
	// default, on.
	Awake *bool `json:"awake,omitempty"`
}

// OutsideOn reports "Reachable from outside home".
func (s Settings) OutsideOn() bool { return s.Outside == nil || *s.Outside }

// AwakeOn reports "Keep this PC awake while it's the server".
func (s Settings) AwakeOn() bool { return s.Awake == nil || *s.Awake }

// LoadSettings reads home-server.json (none is the zero value: off).
func LoadSettings(dataDir string) (Settings, error) {
	raw, err := os.ReadFile(filepath.Join(dataDir, File))
	if errors.Is(err, fs.ErrNotExist) {
		return Settings{}, nil
	}
	if err != nil {
		return Settings{}, err
	}
	var s Settings
	if err := json.Unmarshal(raw, &s); err != nil {
		return Settings{}, errors.New("home: home-server.json is unreadable")
	}
	return s, nil
}

func saveSettings(dataDir string, s Settings) error {
	out, _ := json.MarshalIndent(s, "", "  ")
	path := filepath.Join(dataDir, File)
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, out, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

// Config sets up a Server.
type Config struct {
	DataDir string
	Ident   *identity.Identity
	Chain   *identity.Chain
	Store   *store.Store
	// Servers is the directory the node.addr rows fold into; the server
	// reads its own row back from it to write only on change.
	Servers *mesh.Servers
	// Net carries this device's rooms; the server's hub joins it as a
	// carrier and bridges guest topics onto its LAN carrier.
	Net *roomnet.Transport
	// Wanted reports the §4.5 condition: another certified device, or an
	// open pairing window.
	Wanted func() bool

	// Tests replace these. ListenTCP and ListenUDP bind port N (default:
	// the wildcard address, which is the point of the server: IPv4 only
	// while "Reachable from outside home" is off, IPv4 and IPv6 while it is
	// on, so global IPv6 addresses take connections only when the person
	// asked for that); LANAddrs is this machine's private IPv4 addresses on
	// the home network, the first being where TURN relays from (default:
	// go-room's LocalSubnets on the default gateway's subnet); V6Addrs its
	// global IPv6 addresses; Discover finds the router (default: SSDP, then
	// NAT-PMP at the default gateway); AllowPeer is TURN's peer filter
	// (default: go-room's private-subnet rule).
	ListenTCP func(port int) (net.Listener, error)
	ListenUDP func(port int) (net.PacketConn, error)
	LANAddrs  func() []netip.Addr
	V6Addrs   func() []netip.Addr
	Discover  func(ctx context.Context) ([]portmap.Gateway, error)
	AllowPeer func(user string, ip netip.Addr) bool
	// OnPublicNetwork reports an address on a network Windows has on the
	// Public profile (default: go-room's).
	OnPublicNetwork func(ip netip.Addr) bool
	// Awake holds off idle system sleep while the server runs (default:
	// awake.System()).
	Awake awake.Holder
	// Every is how often addresses are checked for a change (default 30 s).
	Every time.Duration
	Now   func() time.Time
	Logf  func(format string, args ...any)
}

// Server is the home server on this device.
type Server struct {
	cfg  Config
	kick chan struct{}

	mu       sync.Mutex
	set      Settings
	run      *running
	err      string // why the last start failed
	moved    int    // the port N was taken at a start and this is the old one
	last     *mesh.NodeAddr
	lastLoad bool
}

// running is everything a started server holds.
type running struct {
	port   int
	dual   bool // the default listeners took IPv6 too (outside on)
	lanIP  netip.Addr
	tcp    net.Listener
	udp    net.PacketConn
	hub    *room.Hub
	turn   *room.TURNServer
	mapper *portmap.Mapper
	awake  awake.Hold // held while it serves and the setting is on
	// awakeErr is why the hold couldn't be taken.
	awakeErr string
}

// New returns a Server; Run starts it.
func New(cfg Config) *Server {
	if cfg.Logf == nil {
		cfg.Logf = func(string, ...any) {}
	}
	if cfg.Now == nil {
		cfg.Now = time.Now
	}
	if cfg.Every <= 0 {
		cfg.Every = 30 * time.Second
	}
	if cfg.LANAddrs == nil {
		cfg.LANAddrs = lanAddrs
	}
	if cfg.V6Addrs == nil {
		cfg.V6Addrs = v6Addrs
	}
	if cfg.Wanted == nil {
		cfg.Wanted = func() bool { return true }
	}
	if cfg.Awake == nil {
		cfg.Awake = awake.System()
	}
	s := &Server{cfg: cfg, kick: make(chan struct{}, 1)}
	if set, err := LoadSettings(cfg.DataDir); err != nil {
		cfg.Logf("home: %v (the server stays off)", err)
	} else {
		s.set = set
	}
	return s
}

// Kick re-checks at once whether the server should run: a pairing window
// opened or closed, or a setting changed.
func (s *Server) Kick() {
	select {
	case s.kick <- struct{}{}:
	default:
	}
}

// Run keeps the server in the state its setting and §4.5 ask for until
// ctx ends, then stops it (its port mappings are removed).
func (s *Server) Run(ctx context.Context) {
	sub := s.cfg.Store.Subscribe(ctx)
	tick := time.NewTicker(s.cfg.Every)
	defer tick.Stop()
	defer s.stop()
	s.reconcile(ctx)
	for {
		select {
		case <-ctx.Done():
			return
		case ev, ok := <-sub:
			if !ok {
				return
			}
			if ev.Seq == 0 || store.IsPersonKind(ev.Kind) {
				s.dropRevoked()
				s.reconcile(ctx)
			}
		case <-s.kick:
			s.reconcile(ctx)
		case <-tick.C:
			s.reconcile(ctx) // a new DHCP lease, interface or network
		}
	}
}

// Settings is the current setting.
func (s *Server) Settings() Settings {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.set
}

// Set changes the setting (Settings → Devices → Home server, GUI only):
// on, "Reachable from outside home", and a public address for when
// mapping fails ("" for none).
func (s *Server) Set(on, outside bool, public string) error {
	s.mu.Lock()
	set := s.set
	set.On, set.Outside = on, &outside
	if public != "" {
		if _, _, err := splitPublic(public, 1); err != nil {
			s.mu.Unlock()
			return err
		}
	}
	set.Public = public
	if err := saveSettings(s.cfg.DataDir, set); err != nil {
		s.mu.Unlock()
		return err
	}
	s.set = set
	s.mu.Unlock()
	s.Kick()
	return nil
}

// reconcile starts, restarts or stops the server and writes node.addr
// when it changed.
func (s *Server) reconcile(ctx context.Context) {
	if ctx.Err() != nil {
		return
	}
	s.mu.Lock()
	set := s.set
	r := s.run
	s.mu.Unlock()
	if !set.On || !s.cfg.Wanted() {
		if r != nil {
			s.stop()
		}
		if !set.On {
			// Switched off: devices stop dialing it.
			if cur, ok := s.lastRow(); ok && cur.Reach != mesh.ReachOff {
				s.write(ctx, mesh.NodeAddr{V: 1, Reach: mesh.ReachOff})
			}
		}
		return
	}
	if r != nil && s.primary() != r.lanIP {
		// Relayed sockets bind the LAN address, and mappings point at it.
		s.cfg.Logf("home: the LAN address changed, restarting the server")
		s.stop()
		r = nil
	}
	if r != nil && s.defaultListen() && r.dual != set.OutsideOn() {
		// IPv6 is listened on only while outside is on.
		s.stop()
		r = nil
	}
	if r == nil {
		if err := s.start(); err != nil {
			s.cfg.Logf("home: %v", err)
			s.mu.Lock()
			s.err = err.Error()
			s.mu.Unlock()
			return
		}
	}
	s.syncMapper()
	s.syncAwake()
	if a, ok := s.addr(); ok {
		if cur, had := s.lastRow(); !had || !cur.Same(a) {
			s.write(ctx, a)
		}
	}
}

func (s *Server) defaultListen() bool { return s.cfg.ListenTCP == nil && s.cfg.ListenUDP == nil }

// listen binds TCP and UDP port N: the configured listeners, or the
// wildcard address, IPv6 included only when dual.
func (s *Server) listen(port int, dual bool) (net.Listener, net.PacketConn, error) {
	tcpNet, udpNet := "tcp4", "udp4"
	if dual {
		tcpNet, udpNet = "tcp", "udp"
	}
	listenTCP := s.cfg.ListenTCP
	if listenTCP == nil {
		listenTCP = func(port int) (net.Listener, error) { return net.Listen(tcpNet, ":"+strconv.Itoa(port)) }
	}
	listenUDP := s.cfg.ListenUDP
	if listenUDP == nil {
		listenUDP = func(port int) (net.PacketConn, error) { return net.ListenPacket(udpNet, ":"+strconv.Itoa(port)) }
	}
	tcp, err := listenTCP(port)
	if err != nil {
		return nil, nil, err
	}
	udp, err := listenUDP(port)
	if err != nil {
		tcp.Close()
		return nil, nil, err
	}
	return tcp, udp, nil
}

func (s *Server) primary() netip.Addr {
	for _, a := range s.cfg.LANAddrs() {
		if a.Is4() {
			return a
		}
	}
	return netip.Addr{}
}

// start binds port N (another one if N is taken), then the hub, the TURN
// relay, and joins the hub to this device's rooms (§7.1 steps 1 to 4).
func (s *Server) start() error {
	s.mu.Lock()
	set := s.set
	s.mu.Unlock()
	port := set.Port
	if port == 0 {
		port = pickPort()
	}
	var tcp net.Listener
	var udp net.PacketConn
	var lastErr error
	dual := set.OutsideOn()
	for try := range 8 {
		if try > 0 {
			port = pickPort()
		}
		if tcp, udp, lastErr = s.listen(port, dual); lastErr == nil {
			break
		}
	}
	if lastErr != nil {
		return fmt.Errorf("can't listen on a port: %w", lastErr)
	}
	if port != set.Port {
		s.mu.Lock()
		if s.set.Port != 0 {
			s.moved = s.set.Port // a manual port forward must change
		}
		s.set.Port = port
		set = s.set
		s.mu.Unlock()
		if err := saveSettings(s.cfg.DataDir, set); err != nil {
			s.cfg.Logf("home: save %s: %v", File, err)
		}
	}
	hubLn, turnLn := room.SplitTCP(tcp)
	r := &running{port: port, dual: dual, lanIP: s.primary(), tcp: tcp, udp: udp}
	self := s.cfg.Ident.Device.ID
	if r.lanIP.IsValid() {
		key, err := s.cfg.Ident.LoadOrCreateSecret(KeyFile)
		if err != nil {
			tcp.Close()
			udp.Close()
			return fmt.Errorf("TURN key: %w", err)
		}
		ts, err := room.NewTURN(room.TURNConfig{UDP: udp, TCP: turnLn, RelayIP: r.lanIP, Key: key, Realm: Realm,
			Authorize: s.authorizeTURN, AllowPeer: s.cfg.AllowPeer, Logf: s.cfg.Logf})
		if err != nil {
			tcp.Close()
			udp.Close()
			return fmt.Errorf("TURN relay: %w", err)
		}
		r.turn = ts
	} else {
		turnLn.Close()
		udp.Close()
	}
	hub, err := room.NewHub(room.HubConfig{
		Self:     self,
		Identity: room.Identity{Pub: s.cfg.Ident.DevicePub(), Sign: s.cfg.Ident.Sign},
		Authorize: func(id string, pub ed25519.PublicKey) error {
			return mesh.CheckDevice(s.cfg.Chain, self, id, pub)
		},
		// AllowTopic is go-room's default: members may subscribe to the
		// topics this device's own rooms have open (its mesh topics) and
		// to open guest topics, so the hub carries nothing it didn't
		// compute or a certified device didn't open (§8).
		TURN:      r.turn,
		PublicUDP: func() netip.AddrPort { return s.publicUDP(r) },
		Bridge:    s.cfg.Net.LAN(),
		Logf:      s.cfg.Logf,
	})
	if err != nil {
		if r.turn != nil {
			r.turn.Close()
		}
		tcp.Close()
		return fmt.Errorf("hub: %w", err)
	}
	r.hub = hub
	go func() {
		if err := hub.Serve(hubLn); err != nil {
			s.cfg.Logf("home: hub: %v", err)
		}
	}()
	s.cfg.Net.SetLocal(hub, func() []room.ICEServer { return s.ice(r) })
	s.mu.Lock()
	s.run, s.err = r, ""
	s.mu.Unlock()
	s.cfg.Logf("home: the server runs on port %d", port)
	return nil
}

// ice is this device's own STUN and TURN, at its LAN address, with
// credentials it issues itself (§4.4).
func (s *Server) ice(r *running) []room.ICEServer {
	if r.turn == nil {
		return nil
	}
	host := net.JoinHostPort(r.lanIP.String(), strconv.Itoa(r.port))
	c := r.turn.Issue(s.cfg.Ident.Device.ID)
	return []room.ICEServer{
		{URLs: []string{"stun:" + host}},
		{URLs: []string{"turn:" + host + "?transport=udp", "turn:" + host + "?transport=tcp"}, Username: c.Username, Credential: c.Credential},
	}
}

// publicUDP is where the router mapped TURN's UDP port, for a guest that
// dialed the public TCP address (go-room puts it in turn.creds).
func (s *Server) publicUDP(r *running) netip.AddrPort {
	s.mu.Lock()
	m := r.mapper
	s.mu.Unlock()
	if m == nil {
		return netip.AddrPort{}
	}
	st := m.Status()
	if st.State != portmap.StateOK || !st.External.IsValid() {
		return netip.AddrPort{}
	}
	for _, p := range st.Ports {
		if p.Proto == "udp" {
			return netip.AddrPortFrom(st.External, p.External)
		}
	}
	return netip.AddrPort{}
}

// authorizeTURN is asked on every authenticated TURN request, the
// allocation refreshes included: a device that was revoked loses its
// relayed paths at its next refresh (§4.4).
func (s *Server) authorizeTURN(user string) error {
	if _, ok := s.cfg.Chain.Cert(user); !ok {
		return mesh.ErrRefused
	}
	if _, revoked := s.cfg.Chain.Cut(user); revoked {
		return mesh.ErrRefused
	}
	return nil
}

// dropRevoked closes the hub connection of every member that is no longer
// a certified device, and with it the guest topics it opened (§3, Revoke).
func (s *Server) dropRevoked() {
	s.mu.Lock()
	r := s.run
	s.mu.Unlock()
	if r == nil {
		return
	}
	self := s.cfg.Ident.Device.ID
	for _, p := range r.hub.Peers() {
		if p.Guest {
			continue
		}
		if mesh.CheckDevice(s.cfg.Chain, self, p.ID, p.Pub) != nil {
			r.hub.Drop(p.ID)
		}
	}
}

// syncMapper starts or stops port mapping to match "Reachable from outside
// home" (§7.1 step 5). A manual public address needs no mapping.
func (s *Server) syncMapper() {
	s.mu.Lock()
	r, set := s.run, s.set
	if r == nil {
		s.mu.Unlock()
		return
	}
	want := set.OutsideOn() && set.Public == ""
	if want == (r.mapper != nil) {
		s.mu.Unlock()
		return
	}
	if !want {
		m := r.mapper
		r.mapper = nil
		s.mu.Unlock()
		m.Close() // removes the mappings
		return
	}
	n := uint16(r.port)
	ext := func(v uint16) uint16 {
		if v == 0 {
			return n
		}
		return v
	}
	// What an earlier run mapped may still be on the router: a crash or a
	// kill skips the removal, and a permanent lease stays forever. go-room
	// removes these first, if they still point at this PC as "irori".
	var stale []portmap.Port
	for _, p := range []portmap.Port{{Proto: "tcp", External: set.TCPExternal}, {Proto: "udp", External: set.UDPExternal}} {
		if p.External != 0 {
			stale = append(stale, p)
		}
	}
	if s.moved != 0 {
		stale = append(stale, portmap.Port{Proto: "tcp", External: uint16(s.moved)}, portmap.Port{Proto: "udp", External: uint16(s.moved)})
	}
	r.mapper = portmap.Start(portmap.Config{
		Ports:       []portmap.Port{{Proto: "tcp", Internal: n, External: ext(set.TCPExternal)}, {Proto: "udp", Internal: n, External: ext(set.UDPExternal)}},
		Discover:    s.cfg.Discover,
		Stale:       stale,
		Description: "irori",
		OnChange:    func(portmap.Status) { s.Kick() },
		Logf:        s.cfg.Logf,
	})
	s.mu.Unlock()
}

// stop closes everything start opened and removes the port mappings.
func (s *Server) stop() {
	s.mu.Lock()
	r := s.run
	s.run = nil
	s.mu.Unlock()
	if r == nil {
		return
	}
	s.cfg.Net.SetLocal(nil, nil)
	s.mu.Lock()
	hold := r.awake
	r.awake = nil
	s.mu.Unlock()
	if hold != nil {
		hold.Release()
	}
	if r.mapper != nil {
		r.mapper.Close()
	}
	r.hub.Close()
	if r.turn != nil {
		r.turn.Close()
	}
	_ = r.tcp.Close()
	_ = r.udp.Close()
}

// addr is the node.addr row the server's state gives now (§7.1 step 6);
// ok is false while the router is still being asked, so a start doesn't
// write an "unmapped" row only to replace it a moment later.
func (s *Server) addr() (mesh.NodeAddr, bool) {
	s.mu.Lock()
	r, set := s.run, s.set
	var mst portmap.Status
	if r != nil && r.mapper != nil {
		mst = r.mapper.Status()
	}
	s.mu.Unlock()
	if r == nil {
		return mesh.NodeAddr{}, false
	}
	a := mesh.NodeAddr{V: 1, Port: r.port, Reach: mesh.ReachUnmapped, At: s.cfg.Now().UnixMilli()}
	for _, ip := range s.cfg.LANAddrs() {
		if ip.Is4() {
			a.LAN = append(a.LAN, ip.String())
		}
	}
	if set.OutsideOn() {
		// Global IPv6 addresses are dialable from outside: listed (and
		// listened on) only when the person asked for that.
		for _, ip := range s.cfg.V6Addrs() {
			a.V6 = append(a.V6, ip.String())
		}
	}
	switch {
	case set.Public != "":
		host, port, _ := splitPublic(set.Public, r.port)
		a.Public = &mesh.PublicAddr{IP: host, TCP: port, UDP: port}
		a.Via, a.Reach = "manual", mesh.ReachOK
	case r.mapper != nil:
		switch mst.State {
		case portmap.StateOK:
			pub := &mesh.PublicAddr{IP: mst.External.String()}
			for _, p := range mst.Ports {
				if p.Proto == "tcp" {
					pub.TCP = int(p.External)
				} else {
					pub.UDP = int(p.External)
				}
			}
			a.Public, a.Via = pub, mst.Method
			a.Reach = mesh.ReachOK
			if mst.CGNAT {
				a.Reach = mesh.ReachCGNAT
			}
			s.keepExternal(uint16(pub.TCP), uint16(pub.UDP))
		case portmap.StateFailed:
		default:
			return a, false
		}
	}
	return a, true
}

// keepExternal saves the router ports granted, to ask for them again.
func (s *Server) keepExternal(tcp, udp uint16) {
	s.mu.Lock()
	if s.set.TCPExternal == tcp && s.set.UDPExternal == udp {
		s.mu.Unlock()
		return
	}
	s.set.TCPExternal, s.set.UDPExternal = tcp, udp
	set := s.set
	s.mu.Unlock()
	if err := saveSettings(s.cfg.DataDir, set); err != nil {
		s.cfg.Logf("home: save %s: %v", File, err)
	}
}

// lastRow is this device's newest node.addr: the one it wrote this run, or
// the one in the log from an earlier run.
func (s *Server) lastRow() (mesh.NodeAddr, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.last != nil {
		return *s.last, true
	}
	if s.cfg.Servers != nil {
		if a, ok := s.cfg.Servers.Row(s.cfg.Ident.Device.ID); ok {
			s.last = &a
			return a, true
		}
	}
	return mesh.NodeAddr{}, false
}

func (s *Server) write(ctx context.Context, a mesh.NodeAddr) {
	if a.At == 0 {
		a.At = s.cfg.Now().UnixMilli()
	}
	if _, err := s.cfg.Store.Append(ctx, "irori", store.KindNodeAddr, a); err != nil {
		s.cfg.Logf("home: write node.addr: %v", err)
		return
	}
	s.mu.Lock()
	s.last = &a
	s.mu.Unlock()
}

// Reachability, the one-word summary in Devices.
const (
	ReachHome    = "home"    // devices at home reach it; away from home they can't
	ReachOutside = "outside" // reachable from outside home too
	ReachForward = "forward" // away from home needs a port forward (mapping failed)
	ReachShared  = "cgnat"   // the ISP shares the public address: nothing from outside helps
)

// Status is the home server as Devices shows it on the server itself.
type Status struct {
	On      bool   `json:"on"`
	Outside bool   `json:"outside"`
	Public  string `json:"public,omitempty"` // the manual public address, as typed
	Running bool   `json:"running"`
	Port    int    `json:"port,omitempty"`
	// Reach is ReachHome, ReachOutside, ReachForward or ReachShared while
	// it runs.
	Reach string `json:"reach,omitempty"`
	// Address is where a device away from home dials it (shown under the
	// pairing code for a remote join, §3); "" when it isn't reachable.
	Address string `json:"address,omitempty"`
	// Line is the one status line, only when something is wrong.
	Line string `json:"line,omitempty"`
	// Awake is "Keep this PC awake while it's the server"; AwakeLine is
	// "Keeping this PC awake" while it runs and holds it, or why it can't.
	Awake     bool   `json:"awake"`
	AwakeLine string `json:"awakeLine,omitempty"`
}

// Status reports the server for Devices.
func (s *Server) Status() Status {
	s.mu.Lock()
	set, r, errText, moved := s.set, s.run, s.err, s.moved
	var mst portmap.Status
	if r != nil && r.mapper != nil {
		mst = r.mapper.Status()
	}
	awakeLine := ""
	if r != nil && set.AwakeOn() {
		awakeLine = awakeStatus(r)
	}
	s.mu.Unlock()
	st := Status{On: set.On, Outside: set.OutsideOn(), Public: set.Public, Running: r != nil, Port: set.Port,
		Awake: set.AwakeOn(), AwakeLine: awakeLine}
	if !set.On {
		return st
	}
	if r == nil {
		if errText != "" {
			st.Line = "The home server couldn't start: " + errText
		}
		return st
	}
	lan := "this PC"
	if r.lanIP.IsValid() {
		lan = r.lanIP.String()
	}
	a, _ := s.addr()
	st.Address = a.Outside()
	switch {
	case set.Public != "":
		st.Reach = ReachOutside
		st.Line = "Irori can't test the address you entered from inside your home. If your public address changes, it goes stale."
	case !set.OutsideOn():
		st.Reach = ReachHome
	case mst.State == portmap.StateOK && mst.CGNAT:
		st.Reach = ReachShared
		st.Line = "Your internet provider shares your public address (CGNAT), so devices away from home can't reach this PC, and a port forward won't help."
	case mst.State == portmap.StateOK:
		st.Reach = ReachOutside
	case mst.State == portmap.StateFailed:
		st.Reach = ReachForward
		st.Line = fmt.Sprintf("Away from home your devices can't reach this PC. Turn on UPnP on your router, or forward TCP and UDP port %d to %s.", r.port, lan)
	default:
		st.Reach = ReachHome
	}
	if moved != 0 && moved != r.port {
		st.Line = fmt.Sprintf("Port %d was taken, so the server moved to port %d. If you forwarded port %d on your router, forward %d instead.", moved, r.port, moved, r.port)
	}
	if r.lanIP.IsValid() && s.onPublic(r.lanIP) {
		st.Line = "Windows has this network as Public, so it keeps your other devices from reaching this PC. In Windows Settings, open Network & internet and set this network to Private."
	}
	return st
}

func (s *Server) onPublic(ip netip.Addr) bool {
	if s.cfg.OnPublicNetwork != nil {
		return s.cfg.OnPublicNetwork(ip)
	}
	return room.OnPublicNetwork(ip)
}

// splitPublic reads a typed public address: host or host:port.
func splitPublic(in string, port int) (string, int, error) {
	hp, err := roomnet.HostPort(in, port)
	if err != nil {
		return "", 0, err
	}
	h, p, _ := net.SplitHostPort(hp)
	n, _ := strconv.Atoi(p)
	return h, n, nil
}

func pickPort() int { return portMin + rand.IntN(portMax-portMin+1) }

// lanAddrs is this machine's private IPv4 addresses on up interfaces on
// the default gateway's subnet (all of them while the gateway is unknown):
// TURN relays from the first, and virtual adapters (Hyper-V, WSL, Docker,
// VPNs) have private addresses too, which devices on the home network
// can't reach and node.addr needn't carry.
func lanAddrs() []netip.Addr {
	var out []netip.Addr
	var gwSub []netip.Addr
	subs := room.LocalSubnets()
	for _, s := range subs {
		if a := s.Prefix.Addr(); !slices.Contains(out, a) {
			out = append(out, a)
		}
	}
	if gw := gatewayFor(out); gw.IsValid() {
		for _, s := range subs {
			if s.Prefix.Masked().Contains(gw) {
				gwSub = append(gwSub, s.Prefix.Addr())
			}
		}
	}
	if len(gwSub) > 0 {
		return gwSub
	}
	return out
}

// The default gateway is looked up again only when the addresses change:
// on Windows it runs `route print`.
var gwCache struct {
	sync.Mutex
	key string
	gw  netip.Addr
}

func gatewayFor(addrs []netip.Addr) netip.Addr {
	key := fmt.Sprint(addrs)
	gwCache.Lock()
	defer gwCache.Unlock()
	if gwCache.key == key {
		return gwCache.gw
	}
	// A failed lookup isn't cached: the next check (every 30 s) tries
	// again, or TURN could stay on a virtual adapter's address.
	ip, err := gateway.DiscoverGateway()
	if err != nil {
		return netip.Addr{}
	}
	g, ok := netip.AddrFromSlice(ip)
	if !ok {
		return netip.Addr{}
	}
	gwCache.key, gwCache.gw = key, g.Unmap()
	return gwCache.gw
}

// v6Addrs is this machine's global IPv6 addresses (not link-local, not
// unique-local), which a device on the same IPv6 network can dial.
func v6Addrs() []netip.Addr {
	ifs, err := net.Interfaces()
	if err != nil {
		return nil
	}
	var out []netip.Addr
	for _, ifc := range ifs {
		if ifc.Flags&net.FlagUp == 0 || ifc.Flags&net.FlagLoopback != 0 {
			continue
		}
		addrs, _ := ifc.Addrs()
		for _, a := range addrs {
			p, err := netip.ParsePrefix(a.String())
			if err != nil {
				continue
			}
			ip := p.Addr()
			if ip.Is6() && !ip.Is4In6() && ip.IsGlobalUnicast() && !ip.IsPrivate() && !slices.Contains(out, ip) {
				out = append(out, ip)
			}
		}
	}
	return out
}

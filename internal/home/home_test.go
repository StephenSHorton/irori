package home

import (
	"context"
	"crypto/ed25519"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net"
	"net/netip"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	room "github.com/StephenSHorton/go-room"
	"github.com/StephenSHorton/go-room/portmap"

	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/mesh/roomnet"
	"github.com/StephenSHorton/irori/internal/store"
)

// Every listener here is on 127.0.0.1, the router is a fake Gateway in
// process, and nothing reaches the internet.

// router is a fake gateway in process: it maps whatever it is asked for,
// at ext, unless fail is set.
type router struct {
	mu     sync.Mutex
	ext    netip.Addr
	fail   error
	mapped map[string]portmap.Port
}

func (g *router) Method() string { return "upnp" }
func (g *router) ExternalIP(context.Context) (netip.Addr, error) {
	g.mu.Lock()
	defer g.mu.Unlock()
	return g.ext, g.fail
}
func (g *router) Map(_ context.Context, p portmap.Port, life time.Duration) (portmap.Port, time.Duration, error) {
	g.mu.Lock()
	defer g.mu.Unlock()
	if g.fail != nil {
		return portmap.Port{}, 0, g.fail
	}
	if p.External == 0 {
		p.External = p.Internal
	}
	if g.mapped == nil {
		g.mapped = map[string]portmap.Port{}
	}
	g.mapped[p.Proto] = p
	return p, life, nil
}
func (g *router) Unmap(_ context.Context, p portmap.Port) error {
	g.mu.Lock()
	defer g.mu.Unlock()
	delete(g.mapped, p.Proto)
	return nil
}

func (g *router) count() int {
	g.mu.Lock()
	defer g.mu.Unlock()
	return len(g.mapped)
}

type node struct {
	t       *testing.T
	dir     string
	ident   *identity.Identity
	chain   *identity.Chain
	st      *store.Store
	servers *mesh.Servers
	tr      *roomnet.Transport
	srv     *Server
	gw      *router
	wanted  atomic.Bool
	cancel  context.CancelFunc
	done    chan struct{}
}

func newNode(t *testing.T, edit func(*Config)) *node {
	t.Helper()
	n := &node{t: t, dir: t.TempDir(), gw: &router{ext: netip.MustParseAddr("203.0.113.7")}}
	n.wanted.Store(true)
	dv, err := device.LoadOrCreate(n.dir)
	if err != nil {
		t.Fatal(err)
	}
	if n.ident, err = identity.Load(n.dir, dv, identity.FileKeystore{}); err != nil {
		t.Fatal(err)
	}
	n.chain = identity.NewChain(n.ident.Person.ID, n.ident.Person.RootDeviceID, dv.ID)
	if n.st, err = store.OpenWith(context.Background(), filepath.Join(n.dir, "irori.db"), dv.ID, hlc.New(nil),
		store.OpenOptions{Signer: n.ident, Verifier: n.chain}); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { n.st.Close() })
	if err := n.ident.EnsureSelfCert(context.Background(), n.st, n.chain, identity.RoleNode); err != nil {
		t.Fatal(err)
	}
	n.servers = mesh.NewServers(dv.ID, n.chain, n.dir)
	n.tr = roomnet.New(roomnet.Options{Self: dv.ID, Pub: n.ident.DevicePub(), Sign: n.ident.Sign, NoLAN: true, LoopbackOnly: true, Logf: t.Logf})
	t.Cleanup(n.tr.Close)
	cfg := Config{DataDir: n.dir, Ident: n.ident, Chain: n.chain, Store: n.st, Servers: n.servers, Net: n.tr,
		Wanted:    func() bool { return n.wanted.Load() },
		ListenTCP: func(port int) (net.Listener, error) { return net.Listen("tcp4", "127.0.0.1:"+strconv.Itoa(port)) },
		ListenUDP: func(port int) (net.PacketConn, error) {
			return net.ListenPacket("udp4", "127.0.0.1:"+strconv.Itoa(port))
		},
		LANAddrs: func() []netip.Addr { return []netip.Addr{netip.MustParseAddr("127.0.0.1")} },
		V6Addrs:  func() []netip.Addr { return nil },
		Discover: func(context.Context) ([]portmap.Gateway, error) { return []portmap.Gateway{n.gw}, nil },
		Every:    50 * time.Millisecond,
		Logf:     t.Logf}
	if edit != nil {
		edit(&cfg)
	}
	n.srv = New(cfg)
	return n
}

func (n *node) run() {
	ctx, cancel := context.WithCancel(context.Background())
	n.cancel, n.done = cancel, make(chan struct{})
	go func() { defer close(n.done); n.servers.Follow(ctx, n.st) }()
	runDone := make(chan struct{})
	go func() { defer close(runDone); n.srv.Run(ctx) }()
	n.t.Cleanup(func() { cancel(); <-runDone; <-n.done })
}

// rows is every node.addr this device wrote.
func (n *node) rows() []mesh.NodeAddr {
	evs, err := n.st.List(context.Background(), store.Filter{Kinds: []string{store.KindNodeAddr}, Limit: store.MaxListLimit})
	if err != nil {
		n.t.Fatal(err)
	}
	var out []mesh.NodeAddr
	for _, ev := range evs {
		var a mesh.NodeAddr
		_ = json.Unmarshal(ev.Payload, &a)
		out = append(out, a)
	}
	return out
}

func eventually(t *testing.T, what string, ok func() bool) {
	t.Helper()
	deadline := time.Now().Add(15 * time.Second)
	for time.Now().Before(deadline) {
		if ok() {
			return
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for: %s", what)
}

// The server starts, maps its port, and writes node.addr once; renewals and
// re-checks with nothing changed write nothing more. Switching it off
// writes one "off" row and removes the mappings.
func TestServerWritesItsAddressOnlyOnChange(t *testing.T) {
	n := newNode(t, nil)
	n.run()
	if err := n.srv.Set(true, true, ""); err != nil {
		t.Fatal(err)
	}
	eventually(t, "a reachable node.addr", func() bool {
		r := n.rows()
		return len(r) == 1 && r[0].Reach == mesh.ReachOK
	})
	a := n.rows()[0]
	port := n.srv.Settings().Port
	if port < portMin || port > portMax || a.Port != port || a.Via != "upnp" || a.Public == nil ||
		a.Public.IP != "203.0.113.7" || a.Public.TCP != port || a.Public.UDP != port || len(a.LAN) != 1 || a.LAN[0] != "127.0.0.1" {
		t.Fatalf("node.addr %+v (port %d)", a, port)
	}
	st := n.srv.Status()
	if !st.Running || st.Reach != ReachOutside || st.Address != "203.0.113.7:"+strconv.Itoa(port) || st.Line != "" {
		t.Fatalf("status %+v", st)
	}
	time.Sleep(300 * time.Millisecond) // several re-checks
	n.srv.Kick()
	time.Sleep(100 * time.Millisecond)
	if got := len(n.rows()); got != 1 {
		t.Fatalf("%d node.addr rows with nothing changed", got)
	}
	if n.gw.count() != 2 {
		t.Fatalf("%d ports mapped, want TCP and UDP", n.gw.count())
	}
	if err := n.srv.Set(false, true, ""); err != nil {
		t.Fatal(err)
	}
	eventually(t, "an off row", func() bool {
		r := n.rows()
		return len(r) == 2 && r[1].Reach == mesh.ReachOff && len(r[1].Dial()) == 0
	})
	eventually(t, "the mappings removed", func() bool { return n.gw.count() == 0 })
	time.Sleep(200 * time.Millisecond)
	if got := len(n.rows()); got != 2 {
		t.Fatalf("%d rows after switching off", got)
	}
	if n.srv.Status().Running {
		t.Fatal("still running after switching off")
	}
}

// A restart with nothing changed writes no new row: the comparison is with
// the newest row in the log, not with memory.
func TestRestartWritesNoNewRow(t *testing.T) {
	n := newNode(t, nil)
	if err := n.srv.Set(true, true, ""); err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	eventually(t, "the first row", func() bool {
		n.srv.reconcile(ctx) // no Run loop: the test drives it
		return len(n.rows()) == 1
	})
	n.srv.stop()
	if err := n.servers.Load(context.Background(), n.st); err != nil {
		t.Fatal(err)
	}
	again := New(n.srv.cfg)
	eventually(t, "the restart maps again", func() bool {
		again.reconcile(ctx)
		return again.Status().Reach == ReachOutside
	})
	again.reconcile(ctx)
	again.stop()
	if got := len(n.rows()); got != 1 {
		t.Fatalf("%d rows after a restart", got)
	}
}

// The Devices lines on the server come from the mapper's states.
func TestServerStatusLines(t *testing.T) {
	t.Run("mapping failed", func(t *testing.T) {
		n := newNode(t, nil)
		n.gw.fail = errors.New("no IGD")
		n.run()
		_ = n.srv.Set(true, true, "")
		eventually(t, "an unmapped row", func() bool {
			r := n.rows()
			return len(r) == 1 && r[0].Reach == mesh.ReachUnmapped && r[0].Public == nil
		})
		st := n.srv.Status()
		port := strconv.Itoa(st.Port)
		if st.Reach != ReachForward || !strings.Contains(st.Line, "forward TCP and UDP port "+port+" to 127.0.0.1") || st.Address != "" {
			t.Fatalf("status %+v", st)
		}
	})
	t.Run("CGNAT", func(t *testing.T) {
		n := newNode(t, nil)
		n.gw.ext = netip.MustParseAddr("100.64.3.4")
		n.run()
		_ = n.srv.Set(true, true, "")
		eventually(t, "a cgnat row", func() bool {
			r := n.rows()
			return len(r) == 1 && r[0].Reach == mesh.ReachCGNAT && r[0].Outside() == ""
		})
		if st := n.srv.Status(); st.Reach != ReachShared || !strings.Contains(st.Line, "CGNAT") {
			t.Fatalf("status %+v", st)
		}
	})
	t.Run("home only", func(t *testing.T) {
		n := newNode(t, nil)
		n.run()
		_ = n.srv.Set(true, false, "")
		eventually(t, "an unmapped row", func() bool {
			r := n.rows()
			return len(r) == 1 && r[0].Reach == mesh.ReachUnmapped
		})
		if n.gw.count() != 0 {
			t.Fatal("mapped with outside reach off")
		}
		if st := n.srv.Status(); st.Reach != ReachHome || st.Line != "" {
			t.Fatalf("status %+v", st)
		}
	})
	t.Run("manual address", func(t *testing.T) {
		n := newNode(t, nil)
		n.run()
		_ = n.srv.Set(true, true, "pc.example:443")
		eventually(t, "a manual row", func() bool {
			r := n.rows()
			return len(r) == 1 && r[0].Via == "manual" && r[0].Outside() == "pc.example:443"
		})
		if n.gw.count() != 0 {
			t.Fatal("mapped with a manual address")
		}
		if st := n.srv.Status(); st.Reach != ReachOutside || st.Address != "pc.example:443" {
			t.Fatalf("status %+v", st)
		}
		if err := n.srv.Set(true, true, "not an address/"); err == nil {
			t.Fatal("a bad address was taken")
		}
	})
	t.Run("port taken", func(t *testing.T) {
		busy, err := net.Listen("tcp4", "127.0.0.1:0")
		if err != nil {
			t.Fatal(err)
		}
		defer busy.Close()
		old := busy.Addr().(*net.TCPAddr).Port
		n := newNode(t, nil)
		if err := saveSettings(n.dir, Settings{On: true, Port: old}); err != nil {
			t.Fatal(err)
		}
		n.srv = New(n.srv.cfg)
		n.run()
		eventually(t, "a row on a new port", func() bool {
			r := n.rows()
			return len(r) == 1 && r[0].Port != old
		})
		st := n.srv.Status()
		if !strings.Contains(st.Line, "Port "+strconv.Itoa(old)+" was taken") || st.Port == old {
			t.Fatalf("status %+v", st)
		}
		if set, _ := LoadSettings(n.dir); set.Port != st.Port {
			t.Fatalf("the new port isn't saved: %d", set.Port)
		}
	})
}

// A server with no other device and no pairing window binds nothing.
func TestServerWaitsForAnotherDevice(t *testing.T) {
	n := newNode(t, nil)
	n.wanted.Store(false)
	n.run()
	_ = n.srv.Set(true, true, "")
	time.Sleep(300 * time.Millisecond)
	if st := n.srv.Status(); st.Running || len(n.rows()) != 0 || n.gw.count() != 0 {
		t.Fatalf("a lone server started: %+v", st)
	}
	n.wanted.Store(true)
	n.srv.Kick()
	eventually(t, "it starts once wanted", func() bool { return n.srv.Status().Running })
}

// A certified device gets into the hub and gets TURN credentials; once it
// is revoked, its hub connection is dropped and TURN refuses it.
func TestRevokeDropsTheHubConnectionAndTURN(t *testing.T) {
	n := newNode(t, nil)
	ctx := context.Background()
	pub, key, _ := ed25519.GenerateKey(nil)
	const other = "mac00000"
	add, err := n.ident.CertFor(other, hex.EncodeToString(pub), "mac", identity.RoleDesktop, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := n.st.Append(ctx, "irori", store.KindPersonDeviceAdded, add); err != nil {
		t.Fatal(err)
	}
	n.run()
	_ = n.srv.Set(true, false, "")
	eventually(t, "running", func() bool { return n.srv.Status().Running })
	addr := net.JoinHostPort("127.0.0.1", strconv.Itoa(n.srv.Status().Port))
	ident := room.KeyIdentity(key)
	c := room.DialHub(room.HubDialConfig{Self: other, Identity: &ident, Addrs: func() []string { return []string{addr} },
		Expect: func(id string, p ed25519.PublicKey) error {
			if id != n.ident.Device.ID || !p.Equal(n.ident.DevicePub()) {
				return errors.New("not the server")
			}
			return nil
		}})
	defer c.Close()
	eventually(t, "the certified device is in", c.Up)
	tctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	creds, err := c.TURN(tctx)
	if err != nil || !strings.HasSuffix(creds.Username, ":"+other) {
		t.Fatalf("TURN creds %q %v", creds.Username, err)
	}
	if err := n.srv.authorizeTURN(other); err != nil {
		t.Fatalf("TURN refuses a certified device: %v", err)
	}
	rev, err := n.ident.RevokeOf(other, 0, "", "test", time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := n.st.Append(ctx, "irori", store.KindPersonDeviceRevoked, rev); err != nil {
		t.Fatal(err)
	}
	eventually(t, "the revoked device is dropped", func() bool { return !c.Up() })
	if n.srv.authorizeTURN(other) == nil {
		t.Fatal("TURN still admits a revoked device")
	}
	time.Sleep(1500 * time.Millisecond) // the client's reconnect is refused
	if c.Up() {
		t.Fatal("the revoked device got back in")
	}
}

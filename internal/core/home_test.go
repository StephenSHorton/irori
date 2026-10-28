package core

import (
	"context"
	"net"
	"net/netip"
	"os"
	"path/filepath"
	"strconv"
	"sync"
	"testing"
	"time"

	room "github.com/StephenSHorton/go-room"
	"github.com/StephenSHorton/go-room/portmap"

	"github.com/StephenSHorton/irori/internal/home"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/mesh/roomnet"
	"github.com/StephenSHorton/irori/internal/pair"
)

// The multi-Core harness over go-room (§10, "Home server, on 127.0.0.1
// only"): the devices "at home" share a MemLAN (no socket), the home
// server's hub and TURN relay listen on 127.0.0.1, its router is a fake in
// process, and every peer uses loopback candidates only. Nothing binds a
// wildcard address, broadcasts or reaches the internet.

type homeNet struct {
	lan  *room.MemLAN
	mu   sync.Mutex
	next byte
}

func (n *homeNet) conn() net.PacketConn {
	n.mu.Lock()
	n.next++
	b := n.next
	n.mu.Unlock()
	return n.lan.Conn(netip.AddrPortFrom(netip.AddrFrom4([4]byte{10, 0, 0, b}), mesh.LANPort))
}

// loopGateway is a router in process, so no test runs SSDP or NAT-PMP.
type loopGateway struct{}

func (loopGateway) Method() string { return "upnp" }
func (loopGateway) ExternalIP(context.Context) (netip.Addr, error) {
	return netip.MustParseAddr("127.0.0.1"), nil
}
func (loopGateway) Map(_ context.Context, p portmap.Port, life time.Duration) (portmap.Port, time.Duration, error) {
	if p.External == 0 {
		p.External = p.Internal
	}
	return p, life, nil
}
func (loopGateway) Unmap(context.Context, portmap.Port) error { return nil }

// newGoRoomCore is newHCore on go-room: atHome puts it on the MemLAN, and
// away from home it has no LAN carrier at all.
func newGoRoomCore(t *testing.T, hn *homeNet, name, session string, atHome bool) *hcore {
	t.Helper()
	h := &hcore{t: t, name: name, data: t.TempDir(), claude: t.TempDir(), session: session, clk: &clock{}}
	if session != "" {
		dir := filepath.Join(h.claude, "sessions")
		if err := os.MkdirAll(dir, 0o700); err != nil {
			t.Fatal(err)
		}
		writeSession(t, dir, "900001.json", session, name+"-agent", t.TempDir())
	}
	var lanConn net.PacketConn
	if atHome {
		lanConn = hn.conn()
	}
	h.edit = func(o *Options) {
		o.RoomNet = func(ro *roomnet.Options) {
			ro.LoopbackOnly = true
			ro.Logf = func(f string, a ...any) { t.Logf(name+": "+f, a...) }
			if lanConn == nil {
				ro.NoLAN = true
				return
			}
			ro.LANConn = lanConn
			ro.LANTargets = func() []netip.AddrPort { return []netip.AddrPort{hn.lan.Broadcast()} }
		}
		o.Home = func(hc *home.Config) {
			hc.ListenTCP = func(port int) (net.Listener, error) { return net.Listen("tcp4", "127.0.0.1:"+strconv.Itoa(port)) }
			hc.ListenUDP = func(port int) (net.PacketConn, error) {
				return net.ListenPacket("udp4", "127.0.0.1:"+strconv.Itoa(port))
			}
			hc.LANAddrs = func() []netip.Addr { return []netip.Addr{netip.MustParseAddr("127.0.0.1")} }
			hc.V6Addrs = func() []netip.Addr { return nil }
			hc.Discover = func(context.Context) ([]portmap.Gateway, error) { return []portmap.Gateway{loopGateway{}}, nil }
			hc.AllowPeer = func(string, netip.Addr) bool { return true } // relaying between 127.0.0.1 peers
			hc.Every = 100 * time.Millisecond
			hc.Logf = func(f string, a ...any) { t.Logf(name+": "+f, a...) }
		}
	}
	h.start()
	t.Cleanup(h.stop)
	return h
}

// The PC runs the home server. The Mac, away from home (no LAN), pairs by
// typing the server's address beside the code and then syncs through the
// server's hub; the node, at home, pairs by the code alone on the LAN. All
// three share one roster, and the Mac reaches the node only through the
// hub.
func TestCoresOverTheHomeServer(t *testing.T) {
	hn := &homeNet{lan: room.NewMemLAN()}
	pc := newGoRoomCore(t, hn, "pc", "aaaaaaaa-1111-4111-8111-00000000000a", true)
	mac := newGoRoomCore(t, hn, "mac", "bbbbbbbb-2222-4222-8222-00000000000b", false)
	nd := newGoRoomCore(t, hn, "node", "", true)
	// The public address is entered by hand (a router's 127.0.0.1 would
	// read as CGNAT): the address a device away from home dials.
	if err := pc.c.SetHomeServer(true, true, "127.0.0.1"); err != nil {
		t.Fatal(err)
	}
	if pc.c.HomeServer().Running {
		t.Fatal("the server started with no other device and no pairing window")
	}

	// The Mac from away: the Add screen shows the server's outside address.
	st, err := pc.c.Pairing().Add(identity.RoleDesktop)
	if err != nil {
		t.Fatal(err)
	}
	wait(t, "the add screen shows the server's address", func() bool { return pc.c.Pairing().Status().ServerAddr != "" })
	addr := pc.c.Pairing().Status().ServerAddr
	if hs := pc.c.HomeServer(); !hs.Running || hs.Reach != home.ReachOutside || hs.Address != addr {
		t.Fatalf("home server %+v, add screen %q", hs, addr)
	}
	// The address shown is the one the node.addr row gives devices.
	wait(t, "the add screen's address is the row's", func() bool {
		row, ok := pc.c.servers.Row(pc.id())
		return ok && row.Outside() == addr
	})
	if _, err := mac.c.Pairing().JoinVia(st.Code, addr); err != nil {
		t.Fatal(err)
	}
	finishPairing(t, pc, mac, st)
	wait(t, "the mac's rows reach the pc through the hub", synced(pc, mac))

	// The node at home, by the code alone.
	pairCores(t, pc, nd, identity.RoleNode)
	wait(t, "the three converge", synced(pc, mac, nd))
	wait(t, "the mac sees the pc's session", func() bool { return hasCard(mac, pc.agent(), "connected") })
	wait(t, "the node sees the mac's session", func() bool { return hasCard(nd, mac.agent(), "connected") })
	for _, h := range []*hcore{mac, nd} {
		servers := h.c.RemoteServers()
		if len(servers) != 1 || servers[0].DeviceID != pc.id() {
			t.Fatalf("%s's home servers: %+v", h.name, servers)
		}
		wait(t, h.name+" is on the server's hub", func() bool { return h.c.RemoteServers()[0].Up })
	}
	if mac.c.LANError() != "" || len(mac.c.rt.Hubs()) != 1 {
		t.Fatalf("mac: lan %q, hubs %+v", mac.c.LANError(), mac.c.rt.Hubs())
	}

	// Removing the Mac drops its hub connection on the server.
	if err := pc.c.Mesh().Revoke(context.Background(), mac.id(), "test"); err != nil {
		t.Fatal(err)
	}
	wait(t, "the server drops the removed mac", func() bool {
		ups := mac.c.RemoteServers()
		return len(ups) == 0 || !ups[0].Up
	})
}

// finishPairing types the words and the key once b shows them.
func finishPairing(t *testing.T, a, b *hcore, st pair.Status) {
	t.Helper()
	wait(t, b.name+" shows the words", func() bool { return b.c.Pairing().Status().State == pair.StateWords })
	wait(t, a.name+" asks for them", func() bool { return a.c.Pairing().Status().State == pair.StateConfirm })
	words := b.c.Pairing().Status().Words
	if err := a.c.Pairing().Confirm(context.Background(), joinWords(words)); err != nil {
		t.Fatal(err)
	}
	if err := b.c.Pairing().ConfirmKey(context.Background(), st.Fingerprint); err != nil {
		t.Fatal(err)
	}
	wait(t, "paired", func() bool {
		return a.c.Pairing().Status().State == pair.StateDone && b.c.Pairing().Status().State == pair.StateDone
	})
}

func joinWords(w []string) string {
	out := ""
	for i, s := range w {
		if i > 0 {
			out += " "
		}
		out += s
	}
	return out
}

// A hub still connecting within reachGrace isn't unreachable; past it, it
// is, and an up one never is.
func TestRemoteServersGrace(t *testing.T) {
	now := time.Now()
	got := remoteServers([]roomnet.HubState{
		{ID: "new", Since: now.Add(-reachGrace / 2)},
		{ID: "old", Since: now.Add(-2 * reachGrace)},
		{ID: "up", Up: true, Since: now.Add(-2 * reachGrace)},
	}, now)
	if len(got) != 3 || got[0].Unreachable || !got[1].Unreachable || got[2].Unreachable {
		t.Fatalf("remote servers %+v", got)
	}
}

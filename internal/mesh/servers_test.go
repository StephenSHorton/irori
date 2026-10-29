package mesh

import (
	"context"
	"net/netip"
	"slices"
	"testing"
	"time"
)

// The node.addr fold (§4.2): each server's newest row by seq, LAN first,
// "off" drops it, a revoked writer's rows are ignored, the LAN address its
// datagrams came from is dialed first, and pairing hints last only until a
// row arrives. A server's rows reach other devices by sync like any row.
func TestServersFoldNodeAddr(t *testing.T) {
	net := NewFakeNet()
	pc, mac, nd := trio(t, net)
	ctx := context.Background()
	dir := NewServers(mac.id(), mac.chain, mac.dir)
	followCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	go dir.Follow(followCtx, mac.st)
	time.Sleep(50 * time.Millisecond) // subscribed before the rows land

	// Before the node's row: a hint from pairing.
	dir.Hint(nd.id(), []string{"192.168.1.20:24817"})
	if got := dir.Addrs(nd.id()); !slices.Equal(got, []string{"192.168.1.20:24817"}) {
		t.Fatalf("hinted addrs %v", got)
	}
	nd.append("node.addr", NodeAddr{V: 1, Port: 24817, LAN: []string{"192.168.1.20"},
		Public: &PublicAddr{IP: "203.0.113.7", TCP: 24900, UDP: 24901}, Via: "upnp", Reach: ReachOK, At: 1})
	eventually(t, "the node's row reaches the mac", func() bool {
		r, ok := dir.Row(nd.id())
		return ok && r.Port == 24817
	})
	if got := dir.Addrs(nd.id()); !slices.Equal(got, []string{"192.168.1.20:24817", "203.0.113.7:24900"}) {
		t.Fatalf("row addrs %v", got)
	}
	r, _ := dir.Row(nd.id())
	if r.Outside() != "203.0.113.7:24900" {
		t.Fatalf("outside %q", r.Outside())
	}
	// The LAN address its datagrams come from wins.
	dir.Heard(nd.id(), netip.MustParseAddr("192.168.1.33"))
	if got := dir.Addrs(nd.id()); got[0] != "192.168.1.33:24817" {
		t.Fatalf("heard addr not first: %v", got)
	}
	// A device that runs no server is never kept from what it's heard.
	dir.Heard(pc.id(), netip.MustParseAddr("192.168.1.40"))
	for _, s := range dir.List() {
		if s.ID == pc.id() {
			t.Fatal("a device with no server is listed")
		}
	}
	// The last address that worked is kept across restarts.
	dir.Worked(nd.id(), "192.168.1.33:24817")
	again := NewServers(mac.id(), mac.chain, mac.dir)
	if got := again.Addrs(nd.id()); !slices.Contains(got, "192.168.1.33:24817") {
		t.Fatalf("the last good address is lost: %v", got)
	}
	// A newer row replaces it; "off" stops the dialing.
	nd.append("node.addr", NodeAddr{V: 1, Reach: ReachOff, At: 2})
	eventually(t, "off reaches the mac", func() bool { return len(dir.Addrs(nd.id())) == 0 && len(dir.List()) == 0 })

	// A revoked writer's row is ignored.
	pc.append("node.addr", NodeAddr{V: 1, Port: 2000, LAN: []string{"192.168.1.40"}, Reach: ReachUnmapped, At: 3})
	eventually(t, "the pc's row reaches the mac", func() bool { _, ok := dir.Row(pc.id()); return ok })
	if err := nd.node.Revoke(ctx, pc.id(), "test"); err != nil {
		t.Fatal(err)
	}
	eventually(t, "the revoke reaches the mac", func() bool { _, ok := dir.Row(pc.id()); return !ok })
	if len(dir.List()) != 0 {
		t.Fatalf("a revoked server is listed: %+v", dir.List())
	}
}

func TestNodeAddrSameIgnoresWhen(t *testing.T) {
	a := NodeAddr{V: 1, Port: 1, LAN: []string{"10.0.0.2"}, Reach: ReachUnmapped, At: 5}
	b := a
	b.At = 9
	if !a.Same(b) {
		t.Fatal("rows that differ only in At aren't the same")
	}
	b.LAN = []string{"10.0.0.3"}
	if a.Same(b) {
		t.Fatal("a new LAN address is the same")
	}
}

// A node.addr row written after Load but before Follow has subscribed is
// still folded: the home server writes its own row as the net starts, and
// a device it pairs is handed that row's addresses (Hints) to dial its hub.
func TestServersFollowKeepsARowWrittenBeforeItSubscribed(t *testing.T) {
	nd := newDev(t, NewFakeNet(), "node")
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	dir := NewServers(nd.id(), nd.chain, "")
	if err := dir.Load(ctx, nd.st); err != nil {
		t.Fatal(err)
	}
	nd.append("node.addr", NodeAddr{V: 1, Port: 24817, LAN: []string{"127.0.0.1"}, Reach: ReachUnmapped, At: 1})
	done := make(chan struct{})
	go func() { defer close(done); dir.Follow(ctx, nd.st) }()
	eventually(t, "the server's own row is folded", func() bool {
		_, ok := dir.Hints()[nd.id()]
		return ok
	})
	cancel()
	<-done
}

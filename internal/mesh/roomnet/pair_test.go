package roomnet

import (
	"context"
	"strings"
	"testing"
	"time"

	room "github.com/StephenSHorton/go-room"

	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/pair"
	"github.com/StephenSHorton/irori/internal/store"
)

// pair writes go-room's code and room derivation out so that only this
// package imports go-room; they must agree.
func TestPairCodesAreGoRooms(t *testing.T) {
	c := pair.NewCode()
	if got, ok := room.NormalizeCode(c); !ok || got != c {
		t.Fatalf("go-room reads pair's code %q as %q, %v", c, got, ok)
	}
	rc := room.NewCode()
	if got, ok := pair.NormalizeCode(strings.ToLower(rc)); !ok || got != rc {
		t.Fatalf("pair reads go-room's code %q as %q, %v", rc, got, ok)
	}
	want, err := room.SecretFromCode(pair.App, rc)
	if err != nil {
		t.Fatal(err)
	}
	got, err := pair.RoomSecret(rc)
	if err != nil {
		t.Fatal(err)
	}
	if got.Key != want.Key || got.Topic != want.Topic {
		t.Fatalf("pair room %s, go-room %s", got.Topic, want.Topic)
	}
}

func (d *dev) pairer() *pair.Pairer {
	p := pair.New(pair.Options{Net: d.tr, Ident: d.ident, Chain: d.chain, Store: d.st, DataDir: d.dir, Kick: d.node.Kick,
		Servers: d.servers, HelloEvery: 200 * time.Millisecond, Logf: d.logf})
	d.t.Cleanup(p.Cancel)
	return p
}

// pairUp runs one pairing from pa (A) to pb (B) to the end: B joins with
// the code (and server, when set), both reach the words, the person types
// them into A and A's key into B. It returns the code.
func pairUp(t *testing.T, pa, pb *pair.Pairer, server string) string {
	t.Helper()
	ctx := context.Background()
	st, err := pa.Add(identity.RoleDesktop)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := pb.JoinVia(st.Code, server); err != nil {
		t.Fatal(err)
	}
	eventually(t, "both reach the words", 30*time.Second, func() bool {
		return pa.Status().State == pair.StateConfirm && pb.Status().State == pair.StateWords
	})
	if err := pa.Confirm(ctx, strings.Join(pb.Status().Words, " ")); err != nil {
		t.Fatal(err)
	}
	if err := pb.ConfirmKey(ctx, st.Fingerprint); err != nil {
		t.Fatal(err)
	}
	eventually(t, "both paired", 20*time.Second, func() bool {
		return pa.Status().State == pair.StateDone && pb.Status().State == pair.StateDone
	})
	return st.Code
}

// Pairing at home by the code alone (the LAN carrier, no server): A offers
// in the pairing room, both derive the same words from the DTLS
// fingerprints, the words typed into A certify B, and the two reconnect on
// the mesh topic and sync.
func TestPairOnTheLAN(t *testing.T) {
	ctx := context.Background()
	lan := newTestLAN()
	a, b := newDev(t, "pc", ""), newDev(t, "mac", "")
	eb, err := b.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": b.id() + "/before"})
	if err != nil {
		t.Fatal(err)
	}
	a.start(how{lan: lan})
	b.start(how{lan: lan})
	pairUp(t, a.pairer(), b.pairer(), "")
	if b.ident.Current().ID != a.ident.Current().ID {
		t.Fatal("b didn't take a's person")
	}
	eventually(t, "b's earlier row reaches a over the mesh topic", 60*time.Second, func() bool { return a.has(eb) })
}

// Joining from away from home: B has no LAN in common with A, which runs
// the home server. B types the server's address, its hub admits B as a
// guest of the pairing room only, and after pairing B dials the hub as a
// member at the address A's grant named and syncs.
func TestPairAsAGuestOfTheHub(t *testing.T) {
	ctx := context.Background()
	a, b := newDev(t, "pc", ""), newDev(t, "mac", "")
	eb, _ := b.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": b.id() + "/before"})
	a.start(how{host: true})
	a.writeAddr()
	b.start(how{})
	pa, pb := a.pairer(), b.pairer()
	code := pairUp(t, pa, pb, a.hubAddr)
	eventually(t, "b reaches a's hub as a member and syncs", 60*time.Second, func() bool {
		hubs := b.tr.Hubs()
		return a.has(eb) && len(hubs) == 1 && hubs[0].ID == a.id() && hubs[0].Up
	})
	// The pairing topic is closed to guests again: a guest naming it now
	// gets nowhere.
	secret, _ := pair.RoomSecret(code)
	g := room.DialHub(room.HubDialConfig{Self: "stranger", GuestTopic: secret.Topic, Addrs: func() []string { return []string{a.hubAddr} }})
	defer g.Close()
	time.Sleep(500 * time.Millisecond)
	if g.Up() {
		t.Fatal("a guest with no open pairing window got in")
	}
}

// The bridge: A is away (on the hub only), B is at home with only the code
// (on the LAN only), and the home server S carries A's pairing room between
// its LAN and its hub. After pairing, B learns S from A's grant and meets A
// through S's hub.
func TestPairThroughTheServersBridge(t *testing.T) {
	ctx := context.Background()
	lan := newTestLAN()
	s := newDev(t, "node", "")
	a := newDev(t, "pc", s.dir)
	certify(t, s, a)
	b := newDev(t, "mac", "")
	eb, _ := b.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": b.id() + "/before"})
	s.start(how{lan: lan, host: true})
	s.writeAddr()
	a.start(how{dial: map[string]string{s.id(): s.hubAddr}})
	b.start(how{lan: lan})
	eventually(t, "a reaches the server", 60*time.Second, func() bool { return s.peerUp(a.id()) })
	pairUp(t, a.pairer(), b.pairer(), "")
	eventually(t, "b's row reaches a through the server's hub", 90*time.Second, func() bool { return a.has(eb) })
}

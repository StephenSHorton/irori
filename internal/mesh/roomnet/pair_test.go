package roomnet

import (
	"context"
	"strings"
	"testing"
	"time"

	room "github.com/StephenSHorton/go-room"

	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
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

// Pairing over real pion peers on 127.0.0.1 (MemHub signaling, loopback
// candidates only): A offers in the pairing room, both derive the same
// words from the DTLS fingerprints, the words typed into A certify B, and
// the two reconnect on the mesh topic and sync.
func TestPairOverGoRoomOnLoopback(t *testing.T) {
	ctx := context.Background()
	hub := room.NewMemHub()
	a, b := newDev(t, "pc", ""), newDev(t, "mac", "")
	eb, err := b.st.Append(ctx, "irori", store.KindAgentSeen, map[string]any{"key": b.ident.Device.ID + "/before"})
	if err != nil {
		t.Fatal(err)
	}
	a.start(hub)
	b.start(hub)
	mk := func(d *dev) *pair.Pairer {
		return pair.New(pair.Options{Net: d.tr, Ident: d.ident, Chain: d.chain, Store: d.st, DataDir: d.dir, Kick: d.node.Kick,
			HelloEvery: 200 * time.Millisecond, Logf: func(f string, x ...any) { t.Logf(d.name+": "+f, x...) }})
	}
	pa, pb := mk(a), mk(b)
	t.Cleanup(pa.Cancel)
	t.Cleanup(pb.Cancel)
	st, err := pa.Add(identity.RoleDesktop)
	if err != nil {
		t.Fatal(err)
	}
	secret, _ := pair.RoomSecret(st.Code)
	cands := tapLoopback(t, hub, func() map[string][32]byte {
		out := map[string][32]byte{secret.Topic: secret.Key}
		if seed, ok := a.ident.PersonSeed(1); ok {
			s := mesh.MeshSecret(seed, a.ident.Current().ID, 1)
			out[s.Topic] = s.Key
		}
		return out
	})
	if _, err := pb.Join(st.Code); err != nil {
		t.Fatal(err)
	}
	eventually(t, "both reach the words", 30*time.Second, func() bool {
		return pa.Status().State == pair.StateConfirm && pb.Status().State == pair.StateWords
	})
	if *cands == 0 {
		t.Fatal("saw no ICE candidates in the pairing room")
	}
	if err := pa.Confirm(ctx, strings.Join(pb.Status().Words, " ")); err != nil {
		t.Fatal(err)
	}
	if err := pb.ConfirmKey(ctx, st.Fingerprint); err != nil {
		t.Fatal(err)
	}
	eventually(t, "both paired", 20*time.Second, func() bool {
		return pa.Status().State == pair.StateDone && pb.Status().State == pair.StateDone
	})
	if b.ident.Current().ID != a.ident.Current().ID {
		t.Fatal("b didn't take a's person")
	}
	eventually(t, "b's earlier row reaches a over the mesh topic", 60*time.Second, func() bool { return a.has(eb) })
}

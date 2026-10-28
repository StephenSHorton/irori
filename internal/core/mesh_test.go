package core

import (
	"context"
	"encoding/hex"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/store"
)

var testNetIntervals = mesh.Intervals{Presence: 40 * time.Millisecond, Ping: 60 * time.Millisecond, Vector: 80 * time.Millisecond}

func startMeshCore(t *testing.T, net mesh.Transport, data string) *Core {
	t.Helper()
	c, err := Start(context.Background(), Options{DataDir: data, ClaudeDir: t.TempDir(), Version: "0.4.0-test", Headless: true,
		Keystore: identity.FileKeystore{}, Net: net, NetIntervals: testNetIntervals})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	return c
}

// A Core on its own never starts the net layer (§4.5): no topic joined, so
// no hub connection and no UDP socket.
func TestALoneCoreOpensNoNet(t *testing.T) {
	net := mesh.NewFakeNet()
	c := startMeshCore(t, net, t.TempDir())
	time.Sleep(300 * time.Millisecond)
	if net.Joins() != 0 {
		t.Fatalf("a lone Core joined %d topics", net.Joins())
	}
	if st := c.NetStatus(); st.State != mesh.StateOff || len(st.Peers) != 0 {
		t.Fatalf("lone Core net status %+v", st)
	}
}

// Two Cores in one process: once B is certified, both start the net and
// each one's log reaches the other.
func TestTwoCoresReplicateOncePaired(t *testing.T) {
	net := mesh.NewFakeNet()
	ctx := context.Background()
	aDir, bDir := t.TempDir(), t.TempDir()
	a := startMeshCore(t, net, aDir)
	// B joins A's person, as pairing does (§3 step 6): A's person.json, then
	// A's cert for B, then the person rows learned.
	raw, err := os.ReadFile(filepath.Join(aDir, identity.PersonFile))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(bDir, identity.PersonFile), raw, 0o600); err != nil {
		t.Fatal(err)
	}
	b := startMeshCore(t, net, bDir)
	time.Sleep(200 * time.Millisecond)
	if net.Joins() != 0 {
		t.Fatal("the net started before any device was paired")
	}
	add, err := a.ident.CertFor(b.Device().ID, hex.EncodeToString(b.ident.DevicePub()), "mac", identity.RoleDesktop, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := a.store.Append(ctx, "irori", store.KindPersonDeviceAdded, add); err != nil {
		t.Fatal(err)
	}
	person, _ := a.store.List(ctx, store.Filter{Kinds: []string{"person."}})
	for range 2 {
		for _, ev := range person {
			_ = b.chain.Learn(ev)
		}
	}
	b.Mesh().Kick()
	started := func(c *Core) store.Event {
		evs, _ := c.store.List(ctx, store.Filter{Kinds: []string{store.KindDeviceStarted}, DeviceID: c.Device().ID})
		return evs[0]
	}
	sa, sb := started(a), started(b)
	held := func(c *Core, ev store.Event) bool {
		evs, _ := c.store.List(ctx, store.Filter{DeviceID: ev.DeviceID, AfterSeq: ev.Seq - 1, Limit: 1})
		return len(evs) == 1 && evs[0].Seq == ev.Seq
	}
	deadline := time.Now().Add(10 * time.Second)
	for !(held(a, sb) && held(b, sa)) {
		if time.Now().After(deadline) {
			t.Fatal("the two Cores' rows did not cross")
		}
		time.Sleep(20 * time.Millisecond)
	}
	if !a.IsOwner("user:" + b.Device().ID) {
		t.Fatal("a does not count b's owner key")
	}
	var seen bool
	for _, p := range a.NetStatus().Peers {
		seen = seen || p.DeviceID == b.Device().ID
	}
	if a.NetStatus().State != mesh.StateOn || !seen {
		t.Fatalf("a's net status %+v", a.NetStatus())
	}
}

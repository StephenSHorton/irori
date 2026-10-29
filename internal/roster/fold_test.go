package roster

import (
	"context"
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// rig is a store on dev1 that other devices' rows are inserted into.
type rig struct {
	t   *testing.T
	ctx context.Context
	st  *store.Store
	seq map[string]int64
}

func newRig(t *testing.T) *rig {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	st, err := store.Open(ctx, filepath.Join(t.TempDir(), "irori.db"), "dev1", hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	return &rig{t: t, ctx: ctx, st: st, seq: map[string]int64{}}
}

// row inserts dev's next row.
func (g *rig) row(dev, kind string, payload any) {
	g.t.Helper()
	g.seq[dev]++
	raw, _ := json.Marshal(payload)
	at := hlc.Timestamp(time.Now().UnixMilli())<<16 + hlc.Timestamp(g.seq[dev])
	if err := g.st.Insert(g.ctx, store.Event{DeviceID: dev, Seq: g.seq[dev], HLC: at, Actor: "irori", Kind: kind, Payload: raw}); err != nil {
		g.t.Fatal(err)
	}
}

func (g *rig) start(o Options) *Roster {
	g.t.Helper()
	o.Store = g.st
	if o.Self == "" {
		o.Self = "dev1"
	}
	r, err := Start(g.ctx, o)
	if err != nil {
		g.t.Fatal(err)
	}
	return r
}

func keys(cs []presence.AgentCard) []string {
	out := []string{}
	for _, c := range cs {
		out = append(out, c.Key)
	}
	return out
}

func eventually(t *testing.T, what string, ok func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for !ok() {
		if time.Now().After(deadline) {
			t.Fatalf("timed out waiting: %s", what)
		}
		time.Sleep(10 * time.Millisecond)
	}
}

const (
	a2 = "dev2/aaaaaaaa-1111-4111-8111-111111111111"
	b2 = "dev2/bbbbbbbb-2222-4222-8222-222222222222"
	a3 = "dev3/cccccccc-3333-4333-8333-333333333333"
)

// Only an agent's host writes its card: a device writing a card for
// another device's agent is ignored and counted against the writer.
func TestCardFromAnotherDeviceIsIgnored(t *testing.T) {
	g := newRig(t)
	g.row("dev2", store.KindAgentSeen, presence.AgentCard{Key: a2, Agent: "claude", Name: "pc", Status: "busy"})
	g.row("dev2", store.KindAgentSeen, presence.AgentCard{Key: a3, Agent: "claude", Name: "forged"})
	// A status or gone for an agent on another device changes nothing.
	g.row("dev3", store.KindAgentSeen, presence.AgentCard{Key: a3, Agent: "claude", Name: "mac"})
	g.row("dev2", store.KindAgentStatus, map[string]any{"key": a3, "status": "busy", "name": "renamed"})
	g.row("dev2", store.KindAgentGone, map[string]any{"key": a3})
	r := g.start(Options{})

	got := r.Remote()
	if ks := keys(got); len(ks) != 2 || ks[0] != a2 || ks[1] != a3 {
		t.Fatalf("remote cards = %v", ks)
	}
	if got[1].Name != "mac" || got[1].Status != "idle" || got[1].DeviceID != "dev3" {
		t.Fatalf("dev3's card was changed by dev2: %+v", got[1])
	}
	if r.Ignored() != 1 || r.IgnoredBy()["dev2"] != 1 {
		t.Fatalf("ignored = %d by %v, want 1 by dev2", r.Ignored(), r.IgnoredBy())
	}
}

// A card row drops what means nothing on this machine, and a card seen as
// gone comes back idle.
func TestCardRowScrubsLocalFields(t *testing.T) {
	g := newRig(t)
	g.row("dev2", store.KindAgentSeen, presence.AgentCard{Key: a2, Agent: "claude", Name: "pc", Status: "gone",
		LastText: "secret prose", Transcript: `C:\Users\x\t.jsonl`, PID: 42})
	r := g.start(Options{})
	c := r.Remote()[0]
	if c.LastText != "" || c.Transcript != "" || c.PID != 0 || c.Status != "idle" {
		t.Fatalf("card = %+v", c)
	}
}

// agent.status "gone", agent.gone and a newer device.started each retire
// cards; a restart retires only its own device's.
func TestCardsRetire(t *testing.T) {
	g := newRig(t)
	g.row("dev2", store.KindAgentSeen, presence.AgentCard{Key: a2, Agent: "claude"})
	g.row("dev2", store.KindAgentSeen, presence.AgentCard{Key: b2, Agent: "claude"})
	g.row("dev3", store.KindAgentSeen, presence.AgentCard{Key: a3, Agent: "claude"})
	r := g.start(Options{})
	ch := r.Subscribe(g.ctx)
	if ks := keys(r.Remote()); len(ks) != 3 {
		t.Fatalf("cards = %v", ks)
	}

	g.row("dev2", store.KindAgentStatus, map[string]any{"key": a2, "status": "gone"})
	eventually(t, "status gone retires a2", func() bool { return len(r.Remote()) == 2 })
	select {
	case <-ch:
	case <-time.After(5 * time.Second):
		t.Fatal("Subscribe did not signal the change")
	}

	g.row("dev2", store.KindDeviceStarted, map[string]any{})
	eventually(t, "dev2's restart retires b2", func() bool { return len(r.Remote()) == 1 })
	if ks := keys(r.Remote()); ks[0] != a3 {
		t.Fatalf("dev2's restart retired %v", ks)
	}

	g.row("dev3", store.KindAgentGone, map[string]any{"key": a3})
	eventually(t, "agent.gone retires a3", func() bool { return len(r.Remote()) == 0 })
}

// Agents is the local cards then the remote ones; each remote card carries
// its device's liveness, and a revoked device's cards are left out.
func TestAgentsLivenessAndRevoked(t *testing.T) {
	g := newRig(t)
	g.row("dev2", store.KindAgentSeen, presence.AgentCard{Key: a2, Agent: "claude"})
	g.row("dev3", store.KindAgentSeen, presence.AgentCard{Key: a3, Agent: "claude"})
	local := presence.AgentCard{Key: "dev1/dddddddd-4444-4444-8444-444444444444", Agent: "claude"}
	revoked := false
	r := g.start(Options{
		Local:    func() []presence.AgentCard { return []presence.AgentCard{local} },
		Liveness: func(dev string) string { return map[string]string{"dev2": Connected, "dev3": Recent}[dev] },
		Revoked:  func(dev string) bool { return revoked && dev == "dev3" },
	})
	all := r.Agents()
	if ks := keys(all); len(ks) != 3 || ks[0] != local.Key || ks[1] != a2 || ks[2] != a3 {
		t.Fatalf("agents = %v", ks)
	}
	if all[1].DeviceState != Connected || all[2].DeviceState != Recent {
		t.Fatalf("liveness = %q, %q", all[1].DeviceState, all[2].DeviceState)
	}
	revoked = true
	if ks := keys(r.Agents()); len(ks) != 2 || ks[1] != a2 {
		t.Fatalf("with dev3 revoked, agents = %v", ks)
	}

	// No Liveness: every other device counts offline.
	r2 := g.start(Options{})
	for _, c := range r2.Remote() {
		if c.DeviceState != Offline {
			t.Fatalf("no Liveness: %s is %q", c.Key, c.DeviceState)
		}
	}
}

// This device's own rows are presence's: the roster never folds them.
func TestOwnRowsAreNotFolded(t *testing.T) {
	g := newRig(t)
	if _, err := g.st.Append(g.ctx, "irori", store.KindAgentSeen, presence.AgentCard{Key: "dev1/eeeeeeee-5555-4555-8555-555555555555", Agent: "claude"}); err != nil {
		t.Fatal(err)
	}
	r := g.start(Options{})
	if ks := keys(r.Remote()); len(ks) != 0 {
		t.Fatalf("own card folded: %v", ks)
	}
}

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

// A mode another device learned after its agent.seen row arrives on
// agent.status and updates the card; a status row without one (older
// devices, or a mode still unknown) keeps the mode the card had.
func TestAgentStatusCarriesALaterPermissionMode(t *testing.T) {
	ctx := context.Background()
	st, err := store.Open(ctx, filepath.Join(t.TempDir(), "irori.db"), "dev1", hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	defer st.Close()
	const key = "dev2/rrrrrrrr-9999-4999-8999-999999999999"
	var seq int64
	row := func(kind string, payload any) {
		t.Helper()
		seq++
		raw, _ := json.Marshal(payload)
		at := hlc.Timestamp(time.Now().UnixMilli())<<16 + hlc.Timestamp(seq)
		if err := st.Insert(ctx, store.Event{DeviceID: "dev2", Seq: seq, HLC: at, Actor: "irori", Kind: kind, Payload: raw}); err != nil {
			t.Fatal(err)
		}
	}
	mode := func(r *Roster) string {
		for _, c := range r.Remote() {
			if c.Key == key {
				return c.PermissionMode
			}
		}
		t.Fatalf("no card for %s", key)
		return ""
	}
	row(store.KindAgentSeen, presence.AgentCard{Key: key, Agent: "claude", Name: "mac", Status: "idle"})
	row(store.KindAgentStatus, map[string]any{"key": key, "status": "busy", "name": "mac", "permissionMode": "bypassPermissions"})
	r, err := Start(ctx, Options{Store: st, Self: "dev1"})
	if err != nil {
		t.Fatal(err)
	}
	if m := mode(r); m != "bypassPermissions" {
		t.Fatalf("mode after the status row = %q", m)
	}
	row(store.KindAgentStatus, map[string]any{"key": key, "status": "idle", "name": "mac"})
	deadline := time.Now().Add(5 * time.Second)
	for {
		var status string
		for _, c := range r.Remote() {
			if c.Key == key {
				status = c.Status
			}
		}
		if status == "idle" {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("the second status row never folded")
		}
		time.Sleep(10 * time.Millisecond)
	}
	if m := mode(r); m != "bypassPermissions" {
		t.Fatalf("a status row without a mode cleared it: %q", m)
	}
}

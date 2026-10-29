package core

import (
	"context"
	"encoding/json"
	"testing"

	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// A session's mode learned after its agent.seen row reaches the other
// devices on an agent.status row: a new session's first card has no mode
// (its transcript has no mode row yet), and without this the other
// devices showed it as unknown until this Core restarted.
func TestRecordReplicatesAModeLearnedLater(t *testing.T) {
	r := newRig(t)
	c := r.core
	key := c.Device().ID + "/cccccccc-3333-4333-8333-000000000003"
	card := presence.AgentCard{Key: key, DeviceID: c.Device().ID, Agent: "claude", Name: "gamma", Status: "idle"}
	ch := make(chan presence.Change, 4)
	ch <- presence.Change{Kind: presence.Added, Card: card}
	ch <- presence.Change{Kind: presence.Updated, Card: card} // nothing changed: no row
	moded := card
	moded.PermissionMode = "bypassPermissions"
	ch <- presence.Change{Kind: presence.Updated, Card: moded}
	close(ch)
	c.record(context.Background(), ch)

	evs, err := c.store.List(context.Background(), store.Filter{Kinds: []string{store.KindAgentStatus}, Limit: store.MaxListLimit})
	if err != nil {
		t.Fatal(err)
	}
	var rows []map[string]any
	for _, ev := range evs {
		var p map[string]any
		if json.Unmarshal(ev.Payload, &p) == nil && p["key"] == key {
			rows = append(rows, p)
		}
	}
	if len(rows) != 1 || rows[0]["permissionMode"] != "bypassPermissions" || rows[0]["status"] != "idle" || rows[0]["name"] != "gamma" {
		t.Fatalf("status rows for the session: %v", rows)
	}
}

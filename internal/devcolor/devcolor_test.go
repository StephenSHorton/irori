package devcolor

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

var t0 = time.Date(2026, 10, 28, 12, 0, 0, 0, time.UTC)

func devs(ids ...string) []Device {
	out := make([]Device, len(ids))
	for i, id := range ids {
		out[i] = Device{ID: id, At: t0.Add(time.Duration(i) * time.Hour)}
	}
	return out
}

func TestDefaultsAreUniqueWhileThePaletteLasts(t *testing.T) {
	ids := make([]string, len(Palette))
	for i := range ids {
		ids[i] = string(rune('a'+i)) + "-device"
	}
	got := Assign(devs(ids...), nil)
	seen := map[string]bool{}
	for _, id := range ids {
		c := got[id]
		if !Valid(c) || seen[c] {
			t.Fatalf("%s got %q (seen %v)", id, c, seen)
		}
		seen[c] = true
	}
}

func TestAddingADeviceNeverMovesAnOlderOne(t *testing.T) {
	before := Assign(devs("pc", "mac"), nil)
	after := Assign(devs("pc", "mac", "aaa-new"), nil) // sorts first by id, joined last
	if before["pc"] != after["pc"] || before["mac"] != after["mac"] {
		t.Fatalf("before %v after %v", before, after)
	}
}

func TestAPickWinsAndDefaultsAvoidIt(t *testing.T) {
	base := Assign(devs("pc", "mac"), nil)
	// The Mac takes the PC's default: the PC's default moves on, the pick stays.
	got := Assign(devs("pc", "mac"), map[string]string{"mac": base["pc"]})
	if got["mac"] != base["pc"] || got["pc"] == base["pc"] {
		t.Fatalf("got %v from base %v", got, base)
	}
	// An unknown colour is no pick.
	if got := Assign(devs("pc"), map[string]string{"pc": "plaid"}); !Valid(got["pc"]) {
		t.Fatalf("got %v", got)
	}
}

func pickRow(dev string, seq int64, at hlc.Timestamp, actor, device, color string) store.Event {
	raw, _ := json.Marshal(map[string]string{"deviceId": device, "color": color})
	return store.Event{DeviceID: dev, Seq: seq, HLC: at, Actor: actor, Kind: Kind, Payload: raw}
}

func TestPicksKeepTheNewestUserRow(t *testing.T) {
	p := NewPicks()
	p.Apply(pickRow("pc", 1, 10, "user", "mac", "lime"))
	p.Apply(pickRow("mac", 1, 20, "user", "mac", "pink"))
	p.Apply(pickRow("pc", 2, 15, "user", "mac", "cyan")) // older HLC arriving late
	if got := p.Chosen()["mac"]; got != "pink" {
		t.Fatalf("got %q, want the newest pick", got)
	}
	if p.Apply(pickRow("pc", 3, 99, "agent:pc/x", "mac", "sand")) || p.Chosen()["mac"] != "pink" {
		t.Fatal("an agent's row changed the pick")
	}
	if p.Apply(pickRow("pc", 4, 98, "user", "mac", "plaid")) {
		t.Fatal("an unknown colour was applied")
	}
	p.Apply(pickRow("pc", 5, 100, "user", "mac", ""))
	if _, ok := p.Chosen()["mac"]; ok {
		t.Fatal("\"\" didn't go back to the default")
	}
}

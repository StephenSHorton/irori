package core

// Injected into I:internal/core with `go test -overlay` by
// tools/goldens/replay-live.sh (architecture.md §6.3 "Folds", §8 M3): the
// whole Go Core starts on a copy of a data dir, as TestGoOpensRustCopy
// does, with a FakeNet (no hub, no home server, no LAN socket) and an
// empty Claude dir, and writes the same views `irori-dev replay` wrote
// for the Rust Core on an identical copy: the Board (for the person and
// for nobody), leases, the inbox lists and policy, decisions, and every
// timeline in meta.json's key list. It is not `irori serve --headless` on
// purpose: the copy holds this PC's real device identity, and a real
// transport would fork its history on the person's other devices.
//
// Each view is json.Marshal of the service's own value (not the API's
// nil-to-[] wrapper). The Board's clock is pinned to meta.json's "now", as
// the Rust side pins it. go-appended.json lists the kinds of rows the Go
// Core appended on its copy after the Rust views were taken (its start
// row, a self-cert, inbox catch-up holds): those can make a view differ.

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/decision"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/store"
)

func TestGoReplayViews(t *testing.T) {
	home, out := os.Getenv("IRORI_REPLAY_HOME"), os.Getenv("IRORI_REPLAY_OUT")
	if home == "" || out == "" {
		t.Skip("IRORI_REPLAY_HOME / IRORI_REPLAY_OUT not set")
	}
	raw, err := os.ReadFile(filepath.Join(out, "meta.json"))
	if err != nil {
		t.Fatal(err)
	}
	var meta struct {
		Device       string   `json:"device"`
		Seq          int64    `json:"seq"`
		Now          string   `json:"now"`
		TimelineKeys []string `json:"timelineKeys"`
		BoardViewers []string `json:"boardViewers"`
	}
	if err := json.Unmarshal(raw, &meta); err != nil {
		t.Fatal(err)
	}
	now, err := time.Parse(time.RFC3339Nano, meta.Now)
	if err != nil {
		t.Fatal(err)
	}

	c, err := Start(context.Background(), Options{DataDir: home, ClaudeDir: t.TempDir(), Version: "0.5.0-replay-check",
		Headless: true, Net: mesh.NewFakeNet(), Now: func() time.Time { return now }})
	if err != nil {
		t.Fatalf("Go Core did not start on the copy: %v", err)
	}
	defer c.Close()
	if c.device.ID != meta.Device {
		t.Fatalf("Go opened device %s…, Rust replayed %s…", c.device.ID[:8], meta.Device[:8])
	}

	dir := filepath.Join(out, "go")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	write := func(name string, v any) {
		b, err := json.Marshal(v)
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		if err := os.WriteFile(filepath.Join(dir, name), b, 0o600); err != nil {
			t.Fatal(err)
		}
	}

	// Taken first, before the Core's loops run far.
	write("board-user.json", c.coord.Board(coord.BoardFilter{For: "user:" + c.device.ID, DoneLimit: coord.MaxDoneCard}))
	write("board-all.json", c.coord.Board(coord.BoardFilter{DoneLimit: coord.MaxDoneCard}))
	for i, k := range meta.BoardViewers {
		write(fmt.Sprintf("board-agent-%03d.json", i), c.coord.Board(coord.BoardFilter{For: k, DoneLimit: coord.MaxDoneCard}))
	}
	write("leases.json", c.coord.Leases())
	write("inbox-list.json", c.inbox.List(500))
	write("inbox-held.json", c.inbox.HeldHistory(500))
	write("inbox-open.json", c.inbox.Open())
	write("inbox-policy.json", c.inbox.Policy())
	write("decisions.json", c.capture.Decisions(decision.Filter{All: true, Limit: 500}))
	for i, k := range meta.TimelineKeys {
		for _, sub := range []bool{false, true} {
			var pages []capture.Timeline
			q := capture.TimelineQuery{Key: k, Limit: 100, Subagents: sub}
			for range 1000 {
				tl := c.capture.Timeline(q)
				pages = append(pages, tl)
				if !tl.More || len(tl.Entries) == 0 {
					break
				}
				q.Before = tl.Entries[len(tl.Entries)-1].At
			}
			name := "timeline-"
			if sub {
				name = "timeline-sub-"
			}
			write(fmt.Sprintf("%s%03d.json", name, i), pages)
		}
	}

	evs, err := c.store.List(context.Background(), store.Filter{DeviceID: c.device.ID, AfterSeq: meta.Seq, Limit: 1000})
	if err != nil {
		t.Fatal(err)
	}
	kinds := map[string]int{}
	for _, e := range evs {
		kinds[e.Kind]++
	}
	b, _ := json.Marshal(kinds)
	if err := os.WriteFile(filepath.Join(out, "go-appended.json"), b, 0o600); err != nil {
		t.Fatal(err)
	}
	names := make([]string, 0, len(kinds))
	for k := range kinds {
		names = append(names, k)
	}
	sort.Strings(names)
	t.Logf("Go views written: %d timeline keys; Go appended %d rows after seq %d (%v)", len(meta.TimelineKeys), len(evs), meta.Seq, names)
}

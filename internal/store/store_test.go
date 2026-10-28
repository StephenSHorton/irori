package store

import (
	"context"
	"encoding/json"
	"errors"
	"math"
	"path/filepath"
	"reflect"
	"runtime"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
)

func openTest(t *testing.T, dir, device string) *Store {
	t.Helper()
	s, err := Open(context.Background(), filepath.Join(dir, "irori.db"), device, hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { s.Close() })
	return s
}

func mustAppend(t *testing.T, s *Store, kind string, payload any) Event {
	t.Helper()
	ev, err := s.Append(context.Background(), "irori", kind, payload)
	if err != nil {
		t.Fatal(err)
	}
	return ev
}

func all(t *testing.T, s *Store) []Event {
	t.Helper()
	evs, err := s.Missing(context.Background(), Vector{}, MaxMissingLimit)
	if err != nil {
		t.Fatal(err)
	}
	return evs
}

func TestConcurrentAppendsAreGapless(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	const workers, each = 8, 200
	var wg sync.WaitGroup
	errs := make(chan error, workers)
	for w := range workers {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for i := range each {
				if _, err := s.Append(context.Background(), "irori", KindAgentStatus, map[string]int{"w": w, "i": i}); err != nil {
					errs <- err
					return
				}
			}
		}()
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		t.Fatal(err)
	}
	evs, err := s.Missing(context.Background(), Vector{}, workers*each+1)
	if err != nil {
		t.Fatal(err)
	}
	if len(evs) != workers*each {
		t.Fatalf("got %d rows, want %d", len(evs), workers*each)
	}
	for i, ev := range evs {
		if ev.Seq != int64(i+1) {
			t.Fatalf("row %d has seq %d", i, ev.Seq)
		}
		if i > 0 && ev.HLC <= evs[i-1].HLC {
			t.Fatalf("seq %d hlc %d not after seq %d hlc %d", ev.Seq, ev.HLC, evs[i-1].Seq, evs[i-1].HLC)
		}
	}
}

func TestReopenContinuesSeq(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "irori.db")
	s, err := Open(context.Background(), path, "dev-a", hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	mustAppend(t, s, KindDeviceStarted, map[string]string{"name": "one"})
	mustAppend(t, s, KindDeviceStarted, map[string]string{"name": "two"})
	if err := s.Close(); err != nil {
		t.Fatal(err)
	}

	s = openTest(t, dir, "dev-a")
	ev := mustAppend(t, s, KindDeviceStarted, map[string]string{"name": "three"})
	if ev.Seq != 3 {
		t.Fatalf("seq after reopen = %d, want 3", ev.Seq)
	}
	evs := all(t, s)
	if len(evs) != 3 || string(evs[0].Payload) != `{"name":"one"}` {
		t.Fatalf("rows after reopen: %+v", evs)
	}
}

func TestInsertIsIdempotentAndRefusesOwnID(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx := context.Background()
	remote := Event{DeviceID: "dev-b", Seq: 1, HLC: hlc.Timestamp(42) << 16, Actor: "irori", Kind: KindAgentSeen, Payload: json.RawMessage(`{"x":1}`)}
	for range 3 {
		if err := s.Insert(ctx, remote); err != nil {
			t.Fatal(err)
		}
	}
	if evs := all(t, s); len(evs) != 1 || !reflect.DeepEqual(evs[0], remote) {
		t.Fatalf("after repeated insert: %+v", evs)
	}

	own := remote
	own.DeviceID = "dev-a"
	if err := s.Insert(ctx, own); !errors.Is(err, ErrOwnDevice) {
		t.Fatalf("own id insert: err = %v", err)
	}

	gap := remote
	gap.Seq = 3
	if err := s.Insert(ctx, gap); !errors.Is(err, ErrGap) {
		t.Fatalf("gap insert: err = %v", err)
	}
}

func TestInsertObservesClock(t *testing.T) {
	clock := hlc.New(func() time.Time { return time.UnixMilli(1000) })
	s, err := Open(context.Background(), filepath.Join(t.TempDir(), "irori.db"), "dev-a", clock)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	future := hlc.Timestamp(9_000_000) << 16
	if err := s.Insert(context.Background(), Event{DeviceID: "dev-b", Seq: 1, HLC: future, Kind: KindAgentGone}); err != nil {
		t.Fatal(err)
	}
	if ev := mustAppend(t, s, KindAgentGone, nil); ev.HLC <= future {
		t.Fatalf("own hlc %d not after observed %d", ev.HLC, future)
	}
}

func TestHLCRoundTripsAsInt64(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx := context.Background()
	// Year 3000 still fits: ms<<16 stays under 2^63.
	y3000 := hlc.Timestamp(time.Date(3000, 1, 1, 0, 0, 0, 0, time.UTC).UnixMilli()) << 16
	stamps := []hlc.Timestamp{1, hlc.Timestamp(time.Now().UnixMilli()) << 16, y3000, MaxReplicatedHLC}
	for i, ts := range stamps {
		ev := Event{DeviceID: "dev-b", Seq: int64(i + 1), HLC: ts, Kind: KindAgentSeen}
		if err := s.Insert(ctx, ev); err != nil {
			t.Fatal(err)
		}
	}
	evs, err := s.List(ctx, Filter{})
	if err != nil {
		t.Fatal(err)
	}
	for i, ev := range evs {
		if ev.HLC != stamps[i] {
			t.Fatalf("row %d hlc %d, want %d", i, ev.HLC, stamps[i])
		}
	}
	// The clock observed the ceiling and still has room to stamp own rows.
	ev, err := s.Append(ctx, "irori", KindAgentSeen, nil)
	if err != nil {
		t.Fatalf("append after ceiling row: %v", err)
	}
	if ev.HLC <= MaxReplicatedHLC {
		t.Fatalf("append hlc %d not past observed %d", ev.HLC, MaxReplicatedHLC)
	}
}

func TestInsertRefusesHLCPastCeiling(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx := context.Background()
	for _, ts := range []hlc.Timestamp{MaxReplicatedHLC + 1, math.MaxInt64, math.MaxInt64 + 1, math.MaxUint64} {
		if err := s.Insert(ctx, Event{DeviceID: "dev-b", Seq: 1, HLC: ts, Kind: KindAgentSeen}); err == nil {
			t.Fatalf("insert with hlc %d accepted", ts)
		}
	}
	ev, err := s.Append(ctx, "irori", KindAgentSeen, nil)
	if err != nil {
		t.Fatalf("append after refused rows: %v", err)
	}
	if ev.HLC >= MaxReplicatedHLC {
		t.Fatalf("refused rows moved the clock: append hlc %d", ev.HLC)
	}
}

func TestRefusedInsertDoesNotMoveClock(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx := context.Background()
	future := hlc.Timestamp(time.Date(3000, 1, 1, 0, 0, 0, 0, time.UTC).UnixMilli()) << 16
	// Seq 5 on an empty device is a gap.
	if err := s.Insert(ctx, Event{DeviceID: "dev-b", Seq: 5, HLC: future, Kind: KindAgentSeen}); !errors.Is(err, ErrGap) {
		t.Fatalf("got %v, want ErrGap", err)
	}
	// Already held: seq 1 commits; a different row at the same seq is a
	// fork (v0.4 §5.2), kept in forks and never stored or observed.
	if err := s.Insert(ctx, Event{DeviceID: "dev-b", Seq: 1, HLC: 1 << 16, Kind: KindAgentSeen}); err != nil {
		t.Fatal(err)
	}
	if err := s.Insert(ctx, Event{DeviceID: "dev-b", Seq: 1, HLC: future, Kind: KindAgentSeen}); !errors.Is(err, ErrFork) {
		t.Fatalf("got %v, want ErrFork", err)
	}
	ev, err := s.Append(ctx, "irori", KindAgentSeen, nil)
	if err != nil {
		t.Fatal(err)
	}
	if ev.HLC >= future {
		t.Fatalf("refused or re-delivered row moved the clock: append hlc %d >= %d", ev.HLC, future)
	}
}

func TestOpenSeedsClockFromHeldRows(t *testing.T) {
	dir := t.TempDir()
	ctx := context.Background()
	future := hlc.Timestamp(time.Date(3000, 1, 1, 0, 0, 0, 0, time.UTC).UnixMilli()) << 16
	s := openTest(t, dir, "dev-a")
	if err := s.Insert(ctx, Event{DeviceID: "dev-b", Seq: 1, HLC: future, Kind: KindAgentSeen}); err != nil {
		t.Fatal(err)
	}
	s.Close()
	s2 := openTest(t, dir, "dev-a")
	ev, err := s2.Append(ctx, "irori", KindAgentSeen, nil)
	if err != nil {
		t.Fatal(err)
	}
	if ev.HLC <= future {
		t.Fatalf("reopened clock not seeded: append hlc %d <= %d", ev.HLC, future)
	}
}

func TestInsertRefusesInvalidJSONPayload(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx := context.Background()
	bad := Event{DeviceID: "dev-b", Seq: 1, HLC: 1 << 16, Kind: KindAgentSeen, Payload: json.RawMessage(`{not json`)}
	if err := s.Insert(ctx, bad); err == nil {
		t.Fatal("invalid JSON payload accepted")
	}
	good := bad
	good.Payload = json.RawMessage(`{"ok":true}`)
	if err := s.Insert(ctx, good); err != nil {
		t.Fatal(err)
	}
	evs, err := s.List(ctx, Filter{})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := json.Marshal(evs); err != nil {
		t.Fatalf("marshal list: %v", err)
	}
}

func TestSubscribeGoroutineEndsOnClose(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	before := runtime.NumGoroutine()
	for range 100 {
		s.Subscribe(context.Background())
	}
	s.Close()
	deadline := time.Now().Add(2 * time.Second)
	for runtime.NumGoroutine() > before+10 {
		if time.Now().After(deadline) {
			t.Fatalf("goroutines %d after close, started with %d", runtime.NumGoroutine(), before)
		}
		time.Sleep(10 * time.Millisecond)
	}
}

// sync copies what dst lacks from src, a page at a time, the way v0.4 replication will.
func syncOnce(t *testing.T, src, dst *Store) {
	t.Helper()
	ctx := context.Background()
	for {
		v, err := dst.Vector(ctx)
		if err != nil {
			t.Fatal(err)
		}
		evs, err := src.Missing(ctx, v, 7)
		if err != nil {
			t.Fatal(err)
		}
		if len(evs) == 0 {
			return
		}
		for _, ev := range evs {
			if err := dst.Insert(ctx, ev); err != nil {
				t.Fatal(err)
			}
		}
	}
}

func TestReplicationBothWays(t *testing.T) {
	a := openTest(t, t.TempDir(), "dev-a")
	b := openTest(t, t.TempDir(), "dev-b")
	c := openTest(t, t.TempDir(), "dev-c")
	for i := range 20 {
		mustAppend(t, a, KindAgentStatus, map[string]int{"a": i})
	}
	for i := range 13 {
		mustAppend(t, b, KindAgentSeen, map[string]int{"b": i})
	}
	for i := range 5 {
		mustAppend(t, c, KindAgentGone, map[string]int{"c": i})
	}
	// c reaches a only through b.
	syncOnce(t, c, b)
	syncOnce(t, a, b)
	syncOnce(t, b, a)
	syncOnce(t, a, b) // nothing left: must be a no-op

	ra, rb := all(t, a), all(t, b)
	if len(ra) != 38 || !reflect.DeepEqual(ra, rb) {
		t.Fatalf("row sets differ: %d vs %d", len(ra), len(rb))
	}
	va, _ := a.Vector(context.Background())
	vb, _ := b.Vector(context.Background())
	want := Vector{"dev-a": 20, "dev-b": 13, "dev-c": 5}
	if !reflect.DeepEqual(va, want) || !reflect.DeepEqual(vb, want) {
		t.Fatalf("vectors %v %v, want %v", va, vb, want)
	}
	if evs, _ := a.Missing(context.Background(), vb, 0); len(evs) != 0 {
		t.Fatalf("a still has %d rows b lacks", len(evs))
	}
}

func TestMissingRespectsVectorAndLimit(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	for range 5 {
		mustAppend(t, s, KindAgentStatus, nil)
	}
	evs, err := s.Missing(context.Background(), Vector{"dev-a": 2}, 2)
	if err != nil {
		t.Fatal(err)
	}
	if len(evs) != 2 || evs[0].Seq != 3 || evs[1].Seq != 4 {
		t.Fatalf("missing: %+v", evs)
	}
}

func TestListFilters(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx := context.Background()
	started := mustAppend(t, s, KindDeviceStarted, nil)
	seen := mustAppend(t, s, KindAgentSeen, nil)
	status := mustAppend(t, s, KindAgentStatus, nil)
	if err := s.Insert(ctx, Event{DeviceID: "dev-b", Seq: 1, HLC: status.HLC + 1, Kind: KindAgentGone}); err != nil {
		t.Fatal(err)
	}
	// "agent" without the dot is an exact kind and "agentx." must not match "agent.".
	if err := s.Insert(ctx, Event{DeviceID: "dev-b", Seq: 2, HLC: status.HLC + 2, Kind: "agentx.thing"}); err != nil {
		t.Fatal(err)
	}

	kinds := func(evs []Event) []string {
		var out []string
		for _, ev := range evs {
			out = append(out, ev.Kind)
		}
		return out
	}
	cases := []struct {
		name string
		f    Filter
		want []string
	}{
		{"all", Filter{}, []string{KindDeviceStarted, KindAgentSeen, KindAgentStatus, KindAgentGone, "agentx.thing"}},
		{"prefix", Filter{Kinds: []string{"agent."}}, []string{KindAgentSeen, KindAgentStatus, KindAgentGone}},
		{"exact", Filter{Kinds: []string{KindAgentSeen}}, []string{KindAgentSeen}},
		{"no dot is exact", Filter{Kinds: []string{"agent"}}, nil},
		{"mixed", Filter{Kinds: []string{KindDeviceStarted, "agentx."}}, []string{KindDeviceStarted, "agentx.thing"}},
		{"device", Filter{DeviceID: "dev-b"}, []string{KindAgentGone, "agentx.thing"}},
		{"after", Filter{AfterHLC: seen.HLC}, []string{KindAgentStatus, KindAgentGone, "agentx.thing"}},
		{"limit", Filter{Limit: 2}, []string{KindDeviceStarted, KindAgentSeen}},
		{"desc", Filter{Desc: true, Limit: 2}, []string{"agentx.thing", KindAgentGone}},
		{"combined", Filter{Kinds: []string{"agent."}, DeviceID: "dev-a", AfterHLC: started.HLC, Desc: true}, []string{KindAgentStatus, KindAgentSeen}},
	}
	for _, tc := range cases {
		evs, err := s.List(ctx, tc.f)
		if err != nil {
			t.Fatal(err)
		}
		if got := kinds(evs); !reflect.DeepEqual(got, tc.want) {
			t.Errorf("%s: got %v, want %v", tc.name, got, tc.want)
		}
	}
}

func TestListLimitBounds(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx := context.Background()
	for i := range MaxListLimit + 5 {
		if err := s.Insert(ctx, Event{DeviceID: "dev-b", Seq: int64(i + 1), HLC: hlc.Timestamp(i + 1), Kind: KindAgentStatus}); err != nil {
			t.Fatal(err)
		}
	}
	for limit, want := range map[int]int{0: DefaultListLimit, -1: DefaultListLimit, 5000: MaxListLimit} {
		evs, err := s.List(ctx, Filter{Limit: limit})
		if err != nil {
			t.Fatal(err)
		}
		if len(evs) != want {
			t.Errorf("limit %d: got %d rows, want %d", limit, len(evs), want)
		}
	}
}

func TestSubscribeOrder(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx, cancel := context.WithCancel(context.Background())
	before := mustAppend(t, s, KindDeviceStarted, nil)
	ch := s.Subscribe(ctx)

	const n = 50
	var wg sync.WaitGroup
	for w := range 5 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for i := range n / 5 {
				if _, err := s.Append(context.Background(), "irori", KindAgentStatus, []int{w, i}); err != nil {
					t.Error(err)
				}
			}
		}()
	}
	if err := s.Insert(context.Background(), Event{DeviceID: "dev-b", Seq: 1, HLC: 1, Kind: KindAgentSeen}); err != nil {
		t.Fatal(err)
	}
	wg.Wait()

	var got []Event
	for len(got) < n+1 {
		ev := <-ch
		if ev.Seq == 0 {
			t.Fatal("unexpected resync")
		}
		got = append(got, ev)
	}
	var lastSeq int64 = before.Seq
	sawInsert := false
	for _, ev := range got {
		if ev.DeviceID == "dev-b" {
			sawInsert = true
			continue
		}
		if ev.Seq != lastSeq+1 {
			t.Fatalf("delivered seq %d after %d", ev.Seq, lastSeq)
		}
		lastSeq = ev.Seq
	}
	if !sawInsert {
		t.Fatal("inserted row not delivered")
	}
	cancel()
	for range ch {
	}
}

func TestSubscribeResyncOnSlowReader(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	ch := s.Subscribe(ctx)
	total := subBuffer + 40
	for range total {
		mustAppend(t, s, KindAgentStatus, nil) // must not block on the unread channel
	}

	var seqs []int64
	resyncs := 0
	for len(ch) > 0 {
		ev := <-ch
		if ev.Seq == 0 {
			resyncs++
			continue
		}
		seqs = append(seqs, ev.Seq)
	}
	if resyncs != 1 || len(seqs) != subBuffer-1 || seqs[len(seqs)-1] != int64(subBuffer-1) {
		t.Fatalf("resyncs %d, delivered %d (last %v)", resyncs, len(seqs), seqs[len(seqs)-1:])
	}
	// After the resync the reader catches up from the log, then live rows flow again.
	evs, err := s.List(context.Background(), Filter{Limit: MaxListLimit})
	if err != nil || len(evs) != total {
		t.Fatalf("list after resync: %d rows, err %v", len(evs), err)
	}
	next := mustAppend(t, s, KindAgentStatus, nil)
	if ev := <-ch; ev.Seq != next.Seq {
		t.Fatalf("after resync got seq %d, want %d", ev.Seq, next.Seq)
	}
}

func TestSubscribeClosesOnCancelAndClose(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx, cancel := context.WithCancel(context.Background())
	ch := s.Subscribe(ctx)
	cancel()
	if _, ok := <-ch; ok {
		t.Fatal("channel open after cancel")
	}
	ch2 := s.Subscribe(context.Background())
	if err := s.Close(); err != nil {
		t.Fatal(err)
	}
	if _, ok := <-ch2; ok {
		t.Fatal("channel open after Close")
	}
	if _, ok := <-s.Subscribe(context.Background()); ok {
		t.Fatal("subscribe after Close returned an open channel")
	}
	if _, err := s.Append(context.Background(), "irori", KindAgentSeen, nil); !errors.Is(err, ErrClosed) {
		t.Fatalf("append after Close: err = %v", err)
	}
	if err := s.Insert(context.Background(), Event{DeviceID: "dev-b", Seq: 1, Kind: KindAgentSeen}); !errors.Is(err, ErrClosed) {
		t.Fatalf("insert after Close: err = %v", err)
	}
}

func TestOpenCreatesParentDir(t *testing.T) {
	path := filepath.Join(t.TempDir(), "nested", "deeper", "irori.db")
	s, err := Open(context.Background(), path, "dev-a", hlc.New(nil))
	if err != nil {
		t.Fatal(err)
	}
	s.Close()
}

func TestMigrationsRecorded(t *testing.T) {
	dir := t.TempDir()
	s := openTest(t, dir, "dev-a")
	var v int
	if err := s.impl.sql.QueryRow(`SELECT MAX(version) FROM schema_migrations`).Scan(&v); err != nil {
		t.Fatal(err)
	}
	if v != SchemaVersion() {
		t.Fatalf("schema version %d, want %d", v, SchemaVersion())
	}
	// A database from a newer build is refused rather than misread.
	if _, err := s.impl.sql.Exec(`INSERT INTO schema_migrations (version) VALUES (?)`, SchemaVersion()+1); err != nil {
		t.Fatal(err)
	}
	s.Close()
	if _, err := Open(context.Background(), filepath.Join(dir, "irori.db"), "dev-a", hlc.New(nil)); err == nil {
		t.Fatal("opened a newer schema")
	}
	var mode string
	s2, _ := Open(context.Background(), filepath.Join(t.TempDir(), "irori.db"), "dev-a", hlc.New(nil))
	defer s2.Close()
	if err := s2.impl.sql.QueryRow(`PRAGMA journal_mode`).Scan(&mode); err != nil || mode != "wal" {
		t.Fatalf("journal_mode %q err %v", mode, err)
	}
}

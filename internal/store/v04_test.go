package store

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/hlc"
)

type keySigner struct{ key ed25519.PrivateKey }

func newSigner(t *testing.T) keySigner {
	t.Helper()
	_, k, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	return keySigner{k}
}

func (s keySigner) SignRow(msg []byte) ([]byte, error) { return ed25519.Sign(s.key, msg), nil }

func (s keySigner) verifies(ev Event) bool {
	return ed25519.Verify(s.key.Public().(ed25519.PublicKey), SigningBytes(ev), ev.Sig)
}

func TestAppendSignsEveryRow(t *testing.T) {
	sg := newSigner(t)
	s, err := OpenWith(context.Background(), filepath.Join(t.TempDir(), "irori.db"), "dev-a", hlc.New(nil), OpenOptions{Signer: sg})
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	ev := mustAppend(t, s, KindAgentSeen, map[string]string{"key": "dev-a/x"})
	if !sg.verifies(ev) {
		t.Fatal("appended row's signature does not verify")
	}
	held := all(t, s)
	if len(held) != 1 || !sg.verifies(held[0]) {
		t.Fatalf("stored row's signature does not verify: %+v", held)
	}
	// The signature covers the actor: the same row as "user" does not verify.
	forged := held[0]
	forged.Actor = "user"
	if sg.verifies(forged) {
		t.Fatal("signature survived an actor change")
	}
	// Sig stays out of JSON, so /v1/events output is unchanged.
	raw, _ := json.Marshal(held[0])
	if strings.Contains(string(raw), "sig") {
		t.Fatalf("sig in JSON: %s", raw)
	}
}

func TestLocalKindsStayInLocalEvents(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx := context.Background()
	if _, err := s.Append(ctx, "user", KindPolicySet, map[string]any{}); !errors.Is(err, ErrLocalKind) {
		t.Fatalf("Append policy.set: err = %v, want ErrLocalKind", err)
	}
	if _, err := s.AppendLocal(ctx, "irori", KindAgentSeen, nil); !errors.Is(err, ErrNotLocal) {
		t.Fatalf("AppendLocal agent.seen: err = %v, want ErrNotLocal", err)
	}
	mustAppend(t, s, KindAgentSeen, nil)
	for i := range 3 {
		ev, err := s.AppendLocal(ctx, "user", KindPolicySet, map[string]int{"n": i})
		if err != nil {
			t.Fatal(err)
		}
		if ev.Seq != int64(i+1) {
			t.Fatalf("local seq %d, want %d", ev.Seq, i+1)
		}
	}
	if _, err := s.AppendLocal(ctx, "irori", KindMemberJoined, nil); err != nil {
		t.Fatal(err)
	}
	// Vector, Missing and List see only replicated rows.
	if v, _ := s.Vector(ctx); v["dev-a"] != 1 {
		t.Fatalf("vector %v counts local rows", v)
	}
	if evs := all(t, s); len(evs) != 1 || evs[0].Kind != KindAgentSeen {
		t.Fatalf("missing returned %+v", evs)
	}
	loc, err := s.List(ctx, Filter{Local: true, Kinds: []string{KindPolicySet}})
	if err != nil || len(loc) != 3 || string(loc[2].Payload) != `{"n":2}` {
		t.Fatalf("local list %+v err %v", loc, err)
	}
	// A peer can't hand us a local kind.
	if err := s.Insert(ctx, Event{DeviceID: "dev-b", Seq: 1, HLC: 1 << 16, Kind: KindPolicySet}); !errors.Is(err, ErrLocalKind) {
		t.Fatalf("Insert policy.set: err = %v, want ErrLocalKind", err)
	}
}

func TestInsertRecordsForks(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx := context.Background()
	first := Event{DeviceID: "dev-b", Seq: 1, HLC: 5 << 16, Actor: "irori", Kind: KindAgentSeen, Payload: json.RawMessage(`{"a":1}`), Sig: []byte("s1")}
	if err := s.Insert(ctx, first); err != nil {
		t.Fatal(err)
	}
	if err := s.Insert(ctx, first); err != nil {
		t.Fatalf("identical copy: %v", err)
	}
	other := first
	other.Payload = json.RawMessage(`{"a":2}`)
	for range 2 {
		if err := s.Insert(ctx, other); !errors.Is(err, ErrFork) {
			t.Fatalf("different row at a held seq: err = %v, want ErrFork", err)
		}
	}
	forks, err := s.Forks(ctx)
	if err != nil || len(forks) != 1 {
		t.Fatalf("forks %+v err %v", forks, err)
	}
	if string(forks[0].Stored.Payload) != `{"a":1}` || string(forks[0].Other.Payload) != `{"a":2}` {
		t.Fatalf("fork rows %+v", forks[0])
	}
	if evs := all(t, s); len(evs) != 1 || string(evs[0].Payload) != `{"a":1}` {
		t.Fatalf("stored row changed: %+v", evs)
	}
}

func TestInsertRefusesHLCThatDoesNotAdvance(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx := context.Background()
	if err := s.Insert(ctx, Event{DeviceID: "dev-b", Seq: 1, HLC: 10 << 16, Kind: KindAgentSeen}); err != nil {
		t.Fatal(err)
	}
	for _, h := range []hlc.Timestamp{10 << 16, 9 << 16} {
		if err := s.Insert(ctx, Event{DeviceID: "dev-b", Seq: 2, HLC: h, Kind: KindAgentSeen}); !errors.Is(err, ErrHLCOrder) {
			t.Fatalf("hlc %d after %d: err = %v, want ErrHLCOrder", h, 10<<16, err)
		}
	}
	if err := s.Insert(ctx, Event{DeviceID: "dev-b", Seq: 2, HLC: 10<<16 + 1, Kind: KindAgentSeen}); err != nil {
		t.Fatal(err)
	}
}

func TestInsertBatchCommitsUpToTheFirstRefusal(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx := context.Background()
	evs := []Event{
		{DeviceID: "dev-b", Seq: 1, HLC: 1 << 16, Kind: KindAgentSeen},
		{DeviceID: "dev-b", Seq: 2, HLC: 2 << 16, Kind: KindAgentSeen},
		{DeviceID: "dev-b", Seq: 1, HLC: 1 << 16, Kind: KindAgentSeen}, // duplicate: applied
		{DeviceID: "dev-b", Seq: 4, HLC: 4 << 16, Kind: KindAgentSeen}, // gap
		{DeviceID: "dev-b", Seq: 3, HLC: 3 << 16, Kind: KindAgentSeen},
	}
	n, err := s.InsertBatch(ctx, evs)
	if !errors.Is(err, ErrGap) || n != 3 {
		t.Fatalf("InsertBatch = %d, %v; want 3, ErrGap", n, err)
	}
	if v, _ := s.Vector(ctx); v["dev-b"] != 2 {
		t.Fatalf("vector after partial batch %v", v)
	}
}

// Two devices' rows at one HLC straddling a page boundary: the cursor
// returns each exactly once, where AfterHLC would skip the second.
func TestCursorAtPageBoundaryWithEqualHLCs(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	ctx := context.Background()
	tie := hlc.Timestamp(7 << 16)
	for _, dev := range []string{"dev-b", "dev-c", "dev-d"} {
		if err := s.Insert(ctx, Event{DeviceID: dev, Seq: 1, HLC: tie, Kind: KindAgentSeen}); err != nil {
			t.Fatal(err)
		}
	}
	var got []string
	var after Cursor
	for {
		evs, err := s.List(ctx, Filter{After: after, Limit: 2})
		if err != nil {
			t.Fatal(err)
		}
		for _, ev := range evs {
			got = append(got, ev.DeviceID)
			after = ev.Cursor()
		}
		if len(evs) < 2 {
			break
		}
	}
	if strings.Join(got, ",") != "dev-b,dev-c,dev-d" {
		t.Fatalf("paged %v", got)
	}
	evs, _ := s.List(ctx, Filter{AfterHLC: tie - 1, Limit: 2})
	if again, _ := s.List(ctx, Filter{AfterHLC: evs[1].HLC, Limit: 2}); len(again) != 0 {
		t.Fatalf("AfterHLC was expected to skip the tie (the bug the cursor fixes): %+v", again)
	}
}

func TestHoldAppendsUntilOwnRowsReturn(t *testing.T) {
	s := openTest(t, t.TempDir(), "dev-a")
	mustAppend(t, s, KindAgentSeen, nil)
	s.HoldAppends(3)
	if _, err := s.Append(context.Background(), "irori", KindAgentSeen, nil); !errors.Is(err, ErrRestoring) {
		t.Fatalf("append while held: err = %v, want ErrRestoring", err)
	}
	s.HoldAppends(0)
	mustAppend(t, s, KindAgentSeen, nil)
}

// v03DB writes a v0.3 database: schema v1 only, own rows (including the two
// kinds that become device-local) and a WAL left behind.
func v03DB(t *testing.T, path, device string) []Event {
	t.Helper()
	sdb, err := sql.Open("sqlite", path+"?_pragma=journal_mode(WAL)")
	if err != nil {
		t.Fatal(err)
	}
	defer sdb.Close()
	if _, err := sdb.Exec(`CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL DEFAULT (unixepoch()))`); err != nil {
		t.Fatal(err)
	}
	if _, err := sdb.Exec(migrations[0].sql); err != nil {
		t.Fatal(err)
	}
	if _, err := sdb.Exec(`INSERT INTO schema_migrations (version) VALUES (1)`); err != nil {
		t.Fatal(err)
	}
	rows := []Event{
		{Kind: KindDeviceStarted, Actor: "irori", Payload: json.RawMessage(`{"name":"pc","os":"windows","version":"0.3.2"}`)},
		{Kind: KindAgentSeen, Actor: "irori", Payload: json.RawMessage(`{"key":"` + device + `/s1","name":"alpha"}`)},
		{Kind: KindPolicySet, Actor: "user", Payload: json.RawMessage(`{"mode":"allowlist","allow":["alpha"],"block":[],"paused":false}`)},
		{Kind: "message.sent", Actor: "agent:" + device + "/s1", Payload: json.RawMessage(`{"id":"msg_1","text":"hi"}`)},
		{Kind: KindMemberJoined, Actor: "irori", Payload: json.RawMessage(`{"key":"` + device + `/grok-1","name":"grokky"}`)},
		{Kind: KindPolicySet, Actor: "user", Payload: json.RawMessage(`{"mode":"everyone","allow":[],"block":["beta"],"paused":true}`)},
		{Kind: "task.created", Actor: "agent:" + device + "/s1", Payload: json.RawMessage(`{"taskId":"t1","title":"x"}`)},
	}
	for i := range rows {
		rows[i].DeviceID, rows[i].Seq, rows[i].HLC = device, int64(i+1), hlc.Timestamp(int64(i+1)*1000)<<16
		if _, err := sdb.Exec(`INSERT INTO events (device_id, seq, hlc, actor, kind, payload) VALUES (?, ?, ?, ?, ?, ?)`,
			rows[i].DeviceID, rows[i].Seq, int64(rows[i].HLC), rows[i].Actor, rows[i].Kind, []byte(rows[i].Payload)); err != nil {
			t.Fatal(err)
		}
	}
	return rows
}

func fileHash(t *testing.T, path string) [32]byte {
	t.Helper()
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return sha256.Sum256(raw)
}

func TestOneTimeMigrationOfAV03Log(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "irori.db")
	orig := v03DB(t, path, "dev-a")
	sg := newSigner(t)
	ctx := context.Background()

	open := func() *Store {
		t.Helper()
		s, err := OpenWith(ctx, path, "dev-a", hlc.New(nil), OpenOptions{Signer: sg})
		if err != nil {
			t.Fatal(err)
		}
		return s
	}
	s := open()

	// The backup is the v0.3 database, rows and all, still openable as v1.
	backup := path + BackupSuffix
	b, err := sql.Open("sqlite", backup)
	if err != nil {
		t.Fatal(err)
	}
	var ver, n int
	if err := b.QueryRow(`SELECT MAX(version) FROM schema_migrations`).Scan(&ver); err != nil || ver != 1 {
		t.Fatalf("backup schema v%d err %v", ver, err)
	}
	if err := b.QueryRow(`SELECT COUNT(*) FROM events`).Scan(&n); err != nil || n != len(orig) {
		t.Fatalf("backup holds %d rows, want %d (err %v)", n, len(orig), err)
	}
	b.Close()
	sum := fileHash(t, backup)

	// Every own row is signed; the local kinds left local.moved behind and
	// seqs stay gapless.
	evs := all(t, s)
	if len(evs) != len(orig) {
		t.Fatalf("%d rows after migration, want %d", len(evs), len(orig))
	}
	for i, ev := range evs {
		if !sg.verifies(ev) {
			t.Fatalf("row %d not signed: %+v", ev.Seq, ev)
		}
		if ev.Seq != int64(i+1) || ev.HLC != orig[i].HLC || ev.Actor != orig[i].Actor {
			t.Fatalf("row %d changed: %+v vs %+v", i+1, ev, orig[i])
		}
		switch o := orig[i]; {
		case IsLocal(o.Kind):
			if ev.Kind != KindLocalMoved || string(ev.Payload) != `{}` {
				t.Fatalf("seq %d (%s) became %s %s, want local.moved {}", ev.Seq, o.Kind, ev.Kind, ev.Payload)
			}
		case ev.Kind != o.Kind || string(ev.Payload) != string(o.Payload):
			t.Fatalf("seq %d rewritten: %+v", ev.Seq, ev)
		}
	}
	loc, err := s.List(ctx, Filter{Local: true})
	if err != nil || len(loc) != 3 {
		t.Fatalf("local rows %+v err %v", loc, err)
	}
	if loc[0].Kind != KindPolicySet || loc[1].Kind != KindMemberJoined || loc[2].Kind != KindPolicySet ||
		string(loc[2].Payload) != string(orig[5].Payload) || loc[2].HLC != orig[5].HLC {
		t.Fatalf("local rows out of order or changed: %+v", loc)
	}
	// Appends continue after the last v0.3 seq, signed.
	next := mustAppend(t, s, KindAgentStatus, nil)
	if next.Seq != int64(len(orig)+1) || !sg.verifies(next) {
		t.Fatalf("append after migration: %+v", next)
	}
	s.Close()

	// Idempotent: a second start changes nothing and keeps the first backup.
	s2 := open()
	if again := all(t, s2); len(again) != len(orig)+1 {
		t.Fatalf("second open: %d rows", len(again))
	}
	if loc2, _ := s2.List(ctx, Filter{Local: true}); len(loc2) != 3 {
		t.Fatalf("second open moved rows again: %d local", len(loc2))
	}
	s2.Close()
	if fileHash(t, backup) != sum {
		t.Fatal("backup rewritten on a later start")
	}
}

func TestMigrationNeedsASignerAndRefusesUnsignedOwnRows(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "irori.db")
	v03DB(t, path, "dev-a")
	if _, err := Open(context.Background(), path, "dev-a", hlc.New(nil)); err == nil {
		t.Fatal("migrated a v0.3 log without a signer")
	}
	// Nothing was migrated, and the backup that was taken stays the v0.3 copy.
	sg := newSigner(t)
	s, err := OpenWith(context.Background(), path, "dev-a", hlc.New(nil), OpenOptions{Signer: sg})
	if err != nil {
		t.Fatal(err)
	}
	// A row written behind the store's back (unsigned) fails the next start.
	if _, err := s.impl.sql.Exec(`INSERT INTO events (device_id, seq, hlc, actor, kind, payload) VALUES ('dev-a', 99, 1, 'irori', 'agent.seen', 'null')`); err != nil {
		t.Fatal(err)
	}
	s.Close()
	if _, err := OpenWith(context.Background(), path, "dev-a", hlc.New(nil), OpenOptions{Signer: sg}); err == nil || !strings.Contains(err.Error(), "unsigned") {
		t.Fatalf("open with an unsigned own row: err = %v", err)
	}
}

func TestFreshDatabaseTakesNoBackup(t *testing.T) {
	dir := t.TempDir()
	s, err := OpenWith(context.Background(), filepath.Join(dir, "irori.db"), "dev-a", hlc.New(nil), OpenOptions{Signer: newSigner(t)})
	if err != nil {
		t.Fatal(err)
	}
	s.Close()
	if _, err := os.Stat(filepath.Join(dir, "irori.db"+BackupSuffix)); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("fresh database backed up: %v", err)
	}
}

// Rolled back to v0.3 (the backup copied over irori.db), used, then upgraded
// again: the second migration's backup is the database it started from,
// not the first one. No temporary file is left behind.
func TestBackupRetakenAfterARollback(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "irori.db")
	orig := v03DB(t, path, "dev-a")
	sg := newSigner(t)
	ctx := context.Background()
	s, err := OpenWith(ctx, path, "dev-a", hlc.New(nil), OpenOptions{Signer: sg})
	if err != nil {
		t.Fatal(err)
	}
	s.Close()

	// Roll back: the backup replaces the database and its WAL.
	raw, err := os.ReadFile(path + BackupSuffix)
	if err != nil {
		t.Fatal(err)
	}
	for _, f := range []string{path, path + "-wal", path + "-shm"} {
		if err := os.Remove(f); err != nil && !errors.Is(err, os.ErrNotExist) {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(path, raw, 0o600); err != nil {
		t.Fatal(err)
	}
	// v0.3 use: one more row.
	sdb, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := sdb.Exec(`INSERT INTO events (device_id, seq, hlc, actor, kind, payload) VALUES ('dev-a', ?, ?, 'irori', 'agent.seen', '{}')`,
		len(orig)+1, int64(orig[len(orig)-1].HLC)+1<<16); err != nil {
		t.Fatal(err)
	}
	sdb.Close()

	s, err = OpenWith(ctx, path, "dev-a", hlc.New(nil), OpenOptions{Signer: sg})
	if err != nil {
		t.Fatal(err)
	}
	if evs := all(t, s); len(evs) != len(orig)+1 {
		t.Fatalf("%d rows after the second migration, want %d", len(evs), len(orig)+1)
	}
	s.Close()
	b, err := sql.Open("sqlite", path+BackupSuffix)
	if err != nil {
		t.Fatal(err)
	}
	defer b.Close()
	var n int
	if err := b.QueryRow(`SELECT COUNT(*) FROM events`).Scan(&n); err != nil || n != len(orig)+1 {
		t.Fatalf("backup holds %d rows, want %d (err %v)", n, len(orig)+1, err)
	}
	if _, err := os.Stat(path + BackupSuffix + ".tmp"); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("temporary backup left behind: %v", err)
	}
}

// Golden exporter for crates/store (store.md §2.3-2.8, §3, §6).
//
// This file is not part of the Go repo. tools/goldens/export.sh injects it
// into I:internal/store with `go test -overlay` (package store, so it reaches
// the unexported registry, migrations and test helpers). It only runs when
// IRORI_GOLDEN_OUT is set, and writes $IRORI_GOLDEN_OUT/store/*.json:
//
//	signing.json     SigningBytes and RowHash of crafted rows
//	kinds.json       the registry's Side and Board columns, the predicates, board actors
//	schema.json      sqlite_master of a fresh and of a migrated v0.3 database
//	migration.json   a v0.3 log and what the v3 migration makes of it
//	private.json     a script of private-log calls with a fake Verifier, and every answer
//	board.json       the same for a board log
//
// Every key is derived from a fixed seed and every clock is fixed, so the
// files are byte-identical on every run and the Rust replay must reproduce
// each signature, HLC and error text. Byte strings are lowercase hex.
//
// A second, separate test (TestDiffLiveRowHashes) runs only when
// IRORI_DIFF_DB is set: it reads a scratch copy of a real irori.db and writes
// every row's RowHash to IRORI_DIFF_OUT. It never runs from export.sh.
package store

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"sort"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
)

func goldStoreDir(t *testing.T) string {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	return dir
}

func goldWrite(t *testing.T, rel string, v any) {
	t.Helper()
	path := filepath.Join(goldStoreDir(t), "store", rel)
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(v); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

// goldKey is a fixed ed25519 key: seed sha256("irori-rust golden " + name).
func goldKey(name string) ed25519.PrivateKey {
	seed := sha256.Sum256([]byte("irori-rust golden " + name))
	return ed25519.NewKeyFromSeed(seed[:])
}

type goldSigner struct{ key ed25519.PrivateKey }

func (s goldSigner) SignRow(msg []byte) ([]byte, error) { return ed25519.Sign(s.key, msg), nil }

// goldWall is every golden clock's wall time.
const goldWallMs = 1_700_000_000_000

func goldClock() *hlc.Clock { return hlc.New(func() time.Time { return time.UnixMilli(goldWallMs) }) }

func goldNow() time.Time { return time.UnixMilli(goldWallMs) }

// gEvent is an Event with its bytes in hex and its hash.
type gEvent struct {
	DeviceID string `json:"deviceId"`
	Seq      int64  `json:"seq"`
	HLC      uint64 `json:"hlc"`
	Actor    string `json:"actor"`
	Kind     string `json:"kind"`
	Payload  string `json:"payload"`
	Sig      string `json:"sig"`
	RowHash  string `json:"rowHash,omitempty"`
}

func toG(ev Event) gEvent {
	return gEvent{ev.DeviceID, ev.Seq, uint64(ev.HLC), ev.Actor, ev.Kind, hex.EncodeToString(ev.Payload), hex.EncodeToString(ev.Sig), RowHash(ev)}
}

func toGs(evs []Event) []gEvent {
	out := []gEvent{}
	for _, ev := range evs {
		out = append(out, toG(ev))
	}
	return out
}

func errText(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}

// ---------------------------------------------------------------- signing

func TestGoldenStoreSigning(t *testing.T) {
	goldStoreDir(t)
	k := goldKey("dev-a")
	evs := []Event{
		{DeviceID: "dev-a", Seq: 1, HLC: 1 << 16, Actor: "irori", Kind: KindAgentSeen, Payload: json.RawMessage(`{"key":"dev-a/s1"}`)},
		{DeviceID: "0123456789abcdef0123456789abcdef", Seq: 42, HLC: hlc.Timestamp(goldWallMs)<<16 | 7, Actor: "agent:dev/s", Kind: "task.created", Payload: json.RawMessage(`{"taskId":"t1","title":"<b>"}`)},
		{DeviceID: "d", Seq: 1, HLC: 0, Actor: "", Kind: "k", Payload: nil},
		{DeviceID: "d", Seq: 2, HLC: 3, Actor: "", Kind: "k", Payload: json.RawMessage(`null`)},
		{DeviceID: "d", Seq: -1, HLC: MaxReplicatedHLC, Actor: "user", Kind: "x.y", Payload: json.RawMessage(`[]`)},
		{DeviceID: "d", Seq: math.MaxInt64, HLC: math.MaxUint64, Actor: "user", Kind: "x.y", Payload: json.RawMessage(`{}`)},
		{DeviceID: "déviçe", Seq: 7, HLC: 99, Actor: "person:p1 ✓", Kind: "kïnd.ü", Payload: json.RawMessage("\"\xff\xfe\"")},
		{DeviceID: "", Seq: 0, HLC: 0, Actor: "", Kind: "", Payload: json.RawMessage(``)},
	}
	type vec struct {
		Event        gEvent `json:"event"`
		SigningBytes string `json:"signingBytes"`
	}
	var out []vec
	for i, ev := range evs {
		if i%2 == 0 {
			ev.Sig = ed25519.Sign(k, SigningBytes(ev))
		} else if i == 3 {
			ev.Sig = []byte("s1")
		}
		out = append(out, vec{toG(ev), hex.EncodeToString(SigningBytes(ev))})
	}
	goldWrite(t, "signing.json", map[string]any{"vectors": out})
}

// ---------------------------------------------------------------- kinds

func sideName(s Side) string {
	switch s {
	case Replicated:
		return "replicated"
	case Local:
		return "local"
	}
	return "unclassified"
}

func boardName(b Board) string {
	switch b {
	case BoardAccepts:
		return "accepts"
	case BoardRefuses:
		return "refuses"
	}
	return "unclassified"
}

func TestGoldenStoreKinds(t *testing.T) {
	goldStoreDir(t)
	sides := map[string]string{}
	for k, s := range kindSides {
		sides[k] = sideName(s)
	}
	boards := map[string]string{}
	for k, b := range kindBoards {
		boards[k] = boardName(b)
	}
	probe := []string{"", "future.kind", "person.", "person.x", "person", "board.", "board.zzz", "member.", "member.x",
		"agent.card", "agent.cardx", "agent.", "agent", "local.moved", "Task.created", "task.created ", "persons.x"}
	for k := range kindSides {
		probe = append(probe, k)
	}
	sort.Strings(probe)
	type pr struct {
		Kind            string `json:"kind"`
		Side            string `json:"side"`
		Board           string `json:"board"`
		IsLocal         bool   `json:"isLocal"`
		IsPerson        bool   `json:"isPerson"`
		BoardAccepted   bool   `json:"boardAccepted"`
		IsBoardIdentity bool   `json:"isBoardIdentity"`
	}
	var probes []pr
	seen := map[string]bool{}
	for _, k := range probe {
		if seen[k] {
			continue
		}
		seen[k] = true
		probes = append(probes, pr{k, sideName(KindSide(k)), boardName(KindBoard(k)), IsLocal(k), IsPersonKind(k), BoardAccepted(k), IsBoardIdentityKind(k)})
	}
	actors := []string{"irori", "person:p1", "agent:c1", "user", "user:dev-a", "api:dev-a/s1", "agent:dev-a/s1", "agent:", "person:",
		"person:p1/x", "", "Irori", "irori ", "person:a b", "person:a\tb", "person:a\\b", "person:a:b", "agent:x\ny", "agent:x\r",
		"person:" + string(bytes.Repeat([]byte("a"), 128)), "person:" + string(bytes.Repeat([]byte("a"), 129)), "agent:é", "agent:p1 "}
	type ar struct {
		Actor string `json:"actor"`
		OK    bool   `json:"ok"`
	}
	var ars []ar
	for _, a := range actors {
		ars = append(ars, ar{a, BoardActorOK(a)})
	}
	goldWrite(t, "kinds.json", map[string]any{"sides": sides, "boards": boards, "probes": probes, "actors": ars,
		"schemaVersion": SchemaVersion()})
}

// ---------------------------------------------------------------- schema

func goldMaster(t *testing.T, path string) []map[string]string {
	t.Helper()
	sdb, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	defer sdb.Close()
	rows, err := sdb.Query(`SELECT type, name, tbl_name, COALESCE(sql, '') FROM sqlite_master ORDER BY type, name`)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	var out []map[string]string
	for rows.Next() {
		var ty, name, tbl, q string
		if err := rows.Scan(&ty, &name, &tbl, &q); err != nil {
			t.Fatal(err)
		}
		out = append(out, map[string]string{"type": ty, "name": name, "tblName": tbl, "sql": q})
	}
	var versions []int
	vr, err := sdb.Query(`SELECT version FROM schema_migrations ORDER BY version`)
	if err != nil {
		t.Fatal(err)
	}
	defer vr.Close()
	for vr.Next() {
		var v int
		vr.Scan(&v)
		versions = append(versions, v)
	}
	out = append(out, map[string]string{"type": "versions", "name": fmt.Sprint(versions)})
	return out
}

func TestGoldenStoreSchema(t *testing.T) {
	goldStoreDir(t)
	ctx := context.Background()
	fresh := filepath.Join(t.TempDir(), "irori.db")
	s, err := Open(ctx, fresh, "dev-a", goldClock())
	if err != nil {
		t.Fatal(err)
	}
	s.Close()
	migrated := filepath.Join(t.TempDir(), "irori.db")
	v03DB(t, migrated, "dev-a")
	s, err = OpenWith(ctx, migrated, "dev-a", goldClock(), OpenOptions{Signer: goldSigner{goldKey("dev-a")}})
	if err != nil {
		t.Fatal(err)
	}
	s.Close()
	goldWrite(t, "schema.json", map[string]any{"fresh": goldMaster(t, fresh), "migrated": goldMaster(t, migrated),
		"v1": migrations[0].sql, "v2": migrations[1].sql})
}

// ---------------------------------------------------------------- migration

func TestGoldenStoreMigration(t *testing.T) {
	goldStoreDir(t)
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "irori.db")
	orig := v03DB(t, path, "dev-a")
	s, err := OpenWith(ctx, path, "dev-a", goldClock(), OpenOptions{Signer: goldSigner{goldKey("dev-a")}})
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	evs := all(t, s)
	loc, err := s.List(ctx, Filter{Local: true})
	if err != nil {
		t.Fatal(err)
	}
	next, err := s.Append(ctx, "irori", KindAgentStatus, map[string]string{"key": "dev-a/s1", "status": "idle"})
	if err != nil {
		t.Fatal(err)
	}
	goldWrite(t, "migration.json", map[string]any{"v1Rows": toGs(orig), "events": toGs(evs), "local": toGs(loc), "next": toG(next)})
}

// ---------------------------------------------------------------- scripted fake Verifier

// goldVerifier certifies devices by key, like board_test's devVerifier, and
// also answers Cut and CutClaims from what the script sets.
type goldVerifier struct {
	mu       sync.Mutex
	pubs     map[string]ed25519.PublicKey
	cuts     map[string]int64
	claims   []CutClaim
	observed []string
	resets   int
}

func (v *goldVerifier) VerifyRow(ev Event) error {
	v.mu.Lock()
	pub, ok := v.pubs[ev.DeviceID]
	v.mu.Unlock()
	if !ok {
		return fmt.Errorf("%w: %s", ErrNoCert, ev.DeviceID)
	}
	if !ed25519.Verify(pub, SigningBytes(ev), ev.Sig) {
		return fmt.Errorf("%w: %s/%d", ErrBadSig, ev.DeviceID, ev.Seq)
	}
	return nil
}

func (v *goldVerifier) Cut(dev string) (int64, bool) {
	v.mu.Lock()
	defer v.mu.Unlock()
	c, ok := v.cuts[dev]
	return c, ok
}

func (v *goldVerifier) CutClaims() []CutClaim {
	v.mu.Lock()
	defer v.mu.Unlock()
	return append([]CutClaim(nil), v.claims...)
}

func (v *goldVerifier) Observe(ev Event) {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.observed = append(v.observed, fmt.Sprintf("%s/%d %s", ev.DeviceID, ev.Seq, ev.Kind))
}

func (v *goldVerifier) Reset() {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.observed, v.resets = nil, v.resets+1
}

// gOp is one scripted call and what Go answered. The Rust replay makes the
// same call and must give the same answer.
type gOp struct {
	Op      string   `json:"op"`
	Actor   string   `json:"actor,omitempty"`
	Kind    string   `json:"kind,omitempty"`
	Payload string   `json:"payload,omitempty"` // append: raw JSON text
	Events  []gEvent `json:"events,omitempty"`  // insert: the batch
	Dev     string   `json:"dev,omitempty"`
	Seq     int64    `json:"seq,omitempty"`
	Hash    string   `json:"hash,omitempty"`
	Filter  *gFilter `json:"filter,omitempty"`
	Vector  Vector   `json:"vector,omitempty"`
	Limit   int      `json:"limit,omitempty"`

	Err      string       `json:"err"`
	Applied  int          `json:"applied"`
	Out      *gEvent      `json:"out,omitempty"`
	Rows     []gEvent     `json:"rows,omitempty"`
	Forks    []gFork      `json:"forks,omitempty"`
	Got      Vector       `json:"got,omitempty"`
	Counts   *BoardCounts `json:"counts,omitempty"`
	Observed []string     `json:"observed,omitempty"`
}

type gFilter struct {
	Kinds    []string `json:"kinds,omitempty"`
	DeviceID string   `json:"deviceId,omitempty"`
	AfterHLC uint64   `json:"afterHlc,omitempty"`
	After    *gCursor `json:"after,omitempty"`
	AfterSeq int64    `json:"afterSeq,omitempty"`
	Limit    int      `json:"limit,omitempty"`
	Desc     bool     `json:"desc,omitempty"`
	Local    bool     `json:"local,omitempty"`
}

type gCursor struct {
	HLC      uint64 `json:"hlc"`
	DeviceID string `json:"deviceId"`
	Seq      int64  `json:"seq"`
}

type gFork struct {
	Stored    gEvent `json:"stored"`
	Other     gEvent `json:"other"`
	OtherHash string `json:"otherHash"`
	Reason    string `json:"reason"`
}

func (f *gFilter) filter() Filter {
	out := Filter{Kinds: f.Kinds, DeviceID: f.DeviceID, AfterHLC: hlc.Timestamp(f.AfterHLC), AfterSeq: f.AfterSeq,
		Limit: f.Limit, Desc: f.Desc, Local: f.Local}
	if f.After != nil {
		out.After = Cursor{HLC: hlc.Timestamp(f.After.HLC), DeviceID: f.After.DeviceID, Seq: f.After.Seq}
	}
	return out
}

// goldEnv runs a script against one store.
type goldEnv struct {
	t     *testing.T
	path  string
	self  string
	board bool
	ver   *goldVerifier
	s     *Store
	ops   []gOp
	// per-device writers for forged replicated rows
	keys   map[string]ed25519.PrivateKey
	clocks map[string]*hlc.Clock
	seqs   map[string]int64
}

func newGoldEnv(t *testing.T, path string, board bool) *goldEnv {
	e := &goldEnv{t: t, path: path, self: "dev-a", board: board, keys: map[string]ed25519.PrivateKey{},
		clocks: map[string]*hlc.Clock{}, seqs: map[string]int64{}}
	e.ver = &goldVerifier{pubs: map[string]ed25519.PublicKey{}, cuts: map[string]int64{}}
	for _, d := range []string{"dev-a", "dev-b", "dev-c", "dev-x"} {
		e.keys[d] = goldKey(d)
		e.clocks[d] = goldClock()
		if d != "dev-x" {
			e.ver.pubs[d] = e.keys[d].Public().(ed25519.PublicKey)
		}
	}
	e.open()
	return e
}

func (e *goldEnv) open() {
	s, err := OpenWith(context.Background(), e.path, e.self, goldClock(), OpenOptions{
		Signer: goldSigner{e.keys[e.self]}, Verifier: e.ver, Now: goldNow, Board: e.board})
	if err != nil {
		e.t.Fatal(err)
	}
	e.s = s
}

func (e *goldEnv) record(op gOp) {
	if e.board {
		c := e.s.BoardRefused()
		op.Counts = &c
	}
	e.ops = append(e.ops, op)
}

// row forges dev's next row (seq is the next one unless seq > 0), signed
// unless sign is false.
func (e *goldEnv) row(dev string, seq int64, actor, kind, payload string, sign bool) Event {
	if seq == 0 {
		seq = e.seqs[dev] + 1
	}
	ev := Event{DeviceID: dev, Seq: seq, HLC: e.clocks[dev].Now(), Actor: actor, Kind: kind, Payload: json.RawMessage(payload)}
	if sign {
		ev.Sig = ed25519.Sign(e.keys[dev], SigningBytes(ev))
	}
	return ev
}

func (e *goldEnv) append(actor, kind, payload string) {
	ev, err := e.s.Append(context.Background(), actor, kind, json.RawMessage(payload))
	op := gOp{Op: "append", Actor: actor, Kind: kind, Payload: payload, Err: errText(err)}
	if err == nil {
		g := toG(ev)
		op.Out = &g
	}
	e.record(op)
}

func (e *goldEnv) appendLocal(actor, kind, payload string) {
	ev, err := e.s.AppendLocal(context.Background(), actor, kind, json.RawMessage(payload))
	op := gOp{Op: "appendLocal", Actor: actor, Kind: kind, Payload: payload, Err: errText(err)}
	if err == nil {
		g := toG(ev)
		op.Out = &g
	}
	e.record(op)
}

// insert inserts a batch; each device's stream moves past the rows the
// store now holds.
func (e *goldEnv) insert(evs ...Event) {
	n, err := e.s.InsertBatch(context.Background(), evs)
	e.record(gOp{Op: "insert", Events: toGs(evs), Applied: n, Err: errText(err)})
	v, _ := e.s.Vector(context.Background())
	for d := range e.seqs {
		e.seqs[d] = max(e.seqs[d], v[d])
	}
	for d, s := range v {
		e.seqs[d] = max(e.seqs[d], s)
	}
}

// insertRaw inserts one unsigned row exactly as given (bad shapes).
func (e *goldEnv) insertRaw(ev Event) { e.insert(ev) }

func (e *goldEnv) unresolved(ev Event) {
	e.record(gOp{Op: "unresolved", Events: toGs([]Event{ev}), Err: errText(e.s.Unresolved(ev))})
}

func (e *goldEnv) list(f gFilter) {
	evs, err := e.s.List(context.Background(), f.filter())
	e.record(gOp{Op: "list", Filter: &f, Rows: toGs(evs), Err: errText(err)})
}

func (e *goldEnv) snapshot() {
	ctx := context.Background()
	evs, err := e.s.Missing(ctx, Vector{}, MaxMissingLimit)
	if err != nil {
		e.t.Fatal(err)
	}
	e.record(gOp{Op: "missing", Vector: Vector{}, Limit: MaxMissingLimit, Rows: toGs(evs)})
	v, err := e.s.Vector(ctx)
	if err != nil {
		e.t.Fatal(err)
	}
	e.record(gOp{Op: "vector", Got: v})
	ref, err := e.s.Refused(ctx, 0)
	if err != nil {
		e.t.Fatal(err)
	}
	e.record(gOp{Op: "refused", Rows: toGs(ref)})
	forks, err := e.s.Forks(ctx)
	if err != nil {
		e.t.Fatal(err)
	}
	var gf []gFork
	for _, f := range forks {
		gf = append(gf, gFork{toG(f.Stored), toG(f.Other), f.OtherHash, f.Reason})
	}
	e.record(gOp{Op: "forks", Forks: gf})
	e.ver.mu.Lock()
	obs := append([]string(nil), e.ver.observed...)
	e.ver.mu.Unlock()
	e.record(gOp{Op: "observed", Observed: obs})
}

func (e *goldEnv) reopen() {
	e.s.Close()
	e.ver.Reset()
	e.open()
	e.record(gOp{Op: "reopen"})
}

func (e *goldEnv) setCut(dev string, seq int64) {
	e.ver.mu.Lock()
	e.ver.cuts[dev] = seq
	e.ver.mu.Unlock()
	e.record(gOp{Op: "setCut", Dev: dev, Seq: seq})
}

func (e *goldEnv) claim(dev string, seq int64, hash string) {
	e.ver.mu.Lock()
	e.ver.claims = append(e.ver.claims, CutClaim{dev, seq, hash})
	e.ver.mu.Unlock()
	e.record(gOp{Op: "claim", Dev: dev, Seq: seq, Hash: hash})
}

func (e *goldEnv) hold(seq int64) {
	e.s.HoldAppends(seq)
	e.record(gOp{Op: "hold", Seq: seq})
}

func TestGoldenStorePrivate(t *testing.T) {
	goldStoreDir(t)
	e := newGoldEnv(t, filepath.Join(t.TempDir(), "irori.db"), false)
	defer func() { e.s.Close() }()

	e.append("irori", KindDeviceStarted, `{"name":"pc","os":"windows","version":"0.5.0"}`)
	e.append("irori", KindAgentSeen, `{"key":"dev-a/s1","name":"alpha"}`)
	// Device-local rows.
	e.append("user", KindPolicySet, `{}`)
	e.appendLocal("irori", KindAgentSeen, `null`)
	e.appendLocal("user", KindPolicySet, `{"mode":"everyone"}`)
	e.appendLocal("irori", KindMemberJoined, `{"key":"dev-a/grok-1"}`)
	e.appendLocal("user", KindPolicySet, `{"mode":"allowlist","allow":["alpha"]}`)

	// dev-b: an identity row and a plain one.
	b1 := e.row("dev-b", 1, "irori", KindPersonDeviceAdded, `{"cert":{"deviceId":"dev-b"}}`, true)
	b2 := e.row("dev-b", 2, "irori", KindAgentSeen, `{"key":"dev-b/s1"}`, true)
	e.insert(b1, b2)
	e.insert(b2) // identical copy
	// A different signed row at a held seq: a fork, recorded once.
	b2f := e.row("dev-b", 2, "irori", KindAgentSeen, `{"key":"dev-b/other"}`, true)
	e.insert(b2f)
	e.insert(b2f)
	// Unsigned and badly signed copies are refused before any fork is recorded.
	b2u := e.row("dev-b", 2, "irori", KindAgentSeen, `{"key":"dev-b/unsigned"}`, false)
	e.insert(b2u)
	b2x := e.row("dev-b", 2, "irori", KindAgentSeen, `{"key":"dev-b/badsig"}`, false)
	b2x.Sig = ed25519.Sign(e.keys["dev-c"], SigningBytes(b2x))
	e.insert(b2x)
	// The check order on new rows.
	e.insert(e.row("dev-b", 4, "irori", KindAgentSeen, `{}`, true)) // gap
	low := e.row("dev-b", 3, "irori", KindAgentSeen, `{}`, false)
	low.HLC = b2.HLC
	low.Sig = ed25519.Sign(e.keys["dev-b"], SigningBytes(low))
	e.insert(low) // hlc order
	far := e.row("dev-b", 3, "irori", KindAgentSeen, `{}`, false)
	far.HLC = hlc.Timestamp(goldWallMs+3*60*1000) << 16
	far.Sig = ed25519.Sign(e.keys["dev-b"], SigningBytes(far))
	e.insert(far) // ahead
	unsigned3 := e.row("dev-b", 3, "irori", KindAgentSeen, `{}`, false)
	e.insert(unsigned3)
	e.insert(e.row("dev-x", 1, "irori", KindAgentSeen, `{}`, true)) // no cert
	// checkRow's refusals, in its order.
	e.insertRaw(Event{DeviceID: "", Seq: 1, HLC: 1, Kind: KindAgentSeen})
	e.insertRaw(Event{DeviceID: "dev-c", Seq: 0, HLC: 1, Kind: KindAgentSeen})
	e.insertRaw(Event{DeviceID: "dev-c", Seq: -3, HLC: 1, Kind: KindAgentSeen})
	e.insertRaw(Event{DeviceID: "dev-c", Seq: 1, HLC: 1})
	e.insertRaw(Event{DeviceID: "dev-c", Seq: 1, HLC: MaxReplicatedHLC + 1, Kind: KindAgentSeen})
	e.insertRaw(Event{DeviceID: "dev-c", Seq: 1, HLC: math.MaxUint64, Kind: KindAgentSeen})
	e.insertRaw(Event{DeviceID: "dev-c", Seq: 1, HLC: 1, Kind: KindPolicySet})
	e.insertRaw(Event{DeviceID: "dev-c", Seq: 1, HLC: 1, Kind: KindAgentSeen, Payload: json.RawMessage(`{not json`)})
	e.insertRaw(Event{DeviceID: "dev-c", Seq: 1, HLC: 1, Kind: KindAgentSeen, Payload: json.RawMessage(`{"a":1} x`)})
	// An empty payload is stored as null (signed over null).
	c1 := e.row("dev-c", 1, "irori", KindAgentSeen, `null`, true)
	c1.Payload = nil
	e.insert(c1)
	// A batch commits up to its first refusal; duplicates count as applied.
	b3 := e.row("dev-b", 3, "irori", KindAgentStatus, `{"n":3}`, true)
	b4 := e.row("dev-b", 4, "irori", KindAgentStatus, `{"n":4}`, true)
	b6 := e.row("dev-b", 6, "irori", KindAgentStatus, `{"n":6}`, true)
	e.insert(b3, b2, b4, b6, e.row("dev-b", 5, "irori", KindAgentStatus, `{}`, true))
	// This device's own rows come back from a peer (restore): verified like any other.
	e.list(gFilter{DeviceID: "dev-a"})
	own := e.row("dev-a", 3, "irori", KindAgentSeen, `{"key":"dev-a/restored"}`, false)
	own.HLC = hlc.Timestamp(goldWallMs)<<16 | 0xfff0
	own.Sig = ed25519.Sign(e.keys["dev-a"], SigningBytes(own))
	e.insert(own)

	// Hold appends until own rows return.
	e.hold(5)
	e.append("irori", KindAgentSeen, `{}`)
	e.append("irori", KindPersonKeyRotated, `{"epoch":2}`) // never held
	e.hold(0)
	e.append("irori", KindAgentSeen, `{"after":"hold"}`)

	// A revocation cut for dev-b at 3: later rows are refused, rows above it
	// move out when cuts are applied (an identity row, or Open).
	e.setCut("dev-b", 3)
	b5 := e.row("dev-b", 5, "irori", KindAgentStatus, `{"n":5}`, true)
	e.insert(b5) // past cut, checked before the gap
	e.snapshot()
	e.insert(e.row("dev-c", 2, "irori", KindPersonProfile, `{"name":"c"}`, true)) // identity row: cuts apply
	e.snapshot()
	// A revoke past its writer's cut is honoured into refused_events.
	rv := e.row("dev-b", 9, "irori", KindPersonDeviceRevoked, `{"revoke":{"deviceId":"dev-c","cut":2}}`, true)
	e.insert(rv)
	e.insert(rv) // again: INSERT OR IGNORE
	rvUnsigned := e.row("dev-b", 10, "irori", KindPersonDeviceRevoked, `{}`, false)
	e.insert(rvUnsigned) // unsigned revoke past cut: past cut
	rvOld := e.row("dev-b", 11, "irori", KindPersonDeviceRevoked, `{}`, false)
	rvOld.HLC = 1 << 16
	rvOld.Sig = ed25519.Sign(e.keys["dev-b"], SigningBytes(rvOld))
	e.insert(rvOld) // hlc behind its writer's held rows
	rvFar := e.row("dev-b", 12, "irori", KindPersonDeviceRevoked, `{}`, false)
	rvFar.HLC = hlc.Timestamp(goldWallMs+10*60*1000) << 16
	rvFar.Sig = ed25519.Sign(e.keys["dev-b"], SigningBytes(rvFar))
	e.insert(rvFar) // ahead
	// The own device's cut never moves its rows, and Vector does not cap it.
	e.setCut("dev-a", 1)
	// Cut claims: a matching one records nothing, another hash a fork.
	evs, _ := e.s.List(context.Background(), Filter{DeviceID: "dev-c"})
	e.claim("dev-c", 1, RowHash(evs[0]))
	e.claim("dev-c", 2, "00ff")
	e.claim("dev-b", 4, "abcd") // b4 is in refused_events now
	e.claim("dev-z", 1, "dead") // not held
	c3 := e.row("dev-c", 3, "irori", KindPersonProfile, `{"name":"c3"}`, true)
	e.insert(c3)
	// A claim at a row that arrives later is checked as it is inserted.
	e.claim("dev-c", 4, "beef")
	e.insert(e.row("dev-c", 4, "irori", KindAgentSeen, `{}`, true))
	e.snapshot()
	e.reopen()
	e.snapshot()

	// List filters, the cursor and the bounds.
	e.list(gFilter{})
	e.list(gFilter{Kinds: []string{"agent."}})
	e.list(gFilter{Kinds: []string{"agent"}})
	e.list(gFilter{Kinds: []string{"person.", KindDeviceStarted}})
	e.list(gFilter{DeviceID: "dev-b", AfterSeq: 1})
	e.list(gFilter{AfterHLC: uint64(b1.HLC)})
	e.list(gFilter{AfterHLC: math.MaxUint64})
	e.list(gFilter{After: &gCursor{uint64(b1.HLC), "dev-b", 1}, Limit: 3})
	e.list(gFilter{After: &gCursor{uint64(b1.HLC), "dev-b", 1}, Limit: 3, Desc: true})
	e.list(gFilter{After: &gCursor{math.MaxUint64, "", 0}})
	e.list(gFilter{Desc: true, Limit: 2})
	e.list(gFilter{Limit: -1, Kinds: []string{KindAgentSeen}})
	e.list(gFilter{Local: true})
	e.list(gFilter{Local: true, Kinds: []string{KindPolicySet}, Desc: true})
	ctx := context.Background()
	for _, v := range []Vector{{}, {"dev-a": 2, "dev-b": 1}, {"dev-a": 100, "dev-b": 100, "dev-c": 100}} {
		for _, lim := range []int{0, 2, 5} {
			evs, err := e.s.Missing(ctx, v, lim)
			e.record(gOp{Op: "missing", Vector: v, Limit: lim, Rows: toGs(evs), Err: errText(err)})
		}
	}
	e.ver.mu.Lock()
	resets := e.ver.resets
	e.ver.mu.Unlock()
	goldWrite(t, "private.json", map[string]any{"self": e.self, "ops": e.ops, "resets": resets})
}

func TestGoldenStoreBoard(t *testing.T) {
	goldStoreDir(t)
	e := newGoldEnv(t, filepath.Join(t.TempDir(), "boards", "0123456789abcdef0123456789abcdef.db"), true)
	defer func() { e.s.Close() }()
	mj := func(v any) string {
		raw, err := json.Marshal(v)
		if err != nil {
			t.Fatal(err)
		}
		return string(raw)
	}
	insertB := func(actor, kind string, payload any) Event {
		ev := e.row("dev-b", 0, actor, kind, mj(payload), true)
		e.insert(ev)
		return ev
	}

	// The allow-list: every refused kind, and an unknown one.
	var refused []string
	for kind, side := range kindSides {
		if side == Replicated && KindBoard(kind) == BoardRefuses {
			refused = append(refused, kind)
		}
	}
	sort.Strings(refused)
	refused = append(refused, "future.kind")
	for _, kind := range refused {
		e.append("irori", kind, `{}`)
		insertB("irori", kind, map[string]any{})
	}
	for _, kind := range []string{KindPolicySet, KindMemberJoined, KindBoardCardKey, KindBoardPin} {
		e.append("irori", kind, `{}`)
		insertB("irori", kind, map[string]any{})
	}

	// Actors.
	e.append("agent:c1", KindAgentCard, mj(map[string]any{"cardId": "c1", "name": "lead", "kind": "claude", "personId": "p1", "status": "active"}))
	for _, actor := range []string{"irori", "person:p1", "agent:c1", "user", "user:dev-a", "user:anything", "api:dev-a/s1",
		"agent:dev-a/s1", "agent:", "person:", "person:p1/x", ""} {
		row := mj(map[string]any{"taskId": "t-" + actor, "title": "x", "by": ""})
		e.append(actor, "task.created", row)
		insertB(actor, "task.created", map[string]any{"taskId": "t-" + actor, "title": "x", "by": ""})
	}
	e.append("agent:c1", KindAgentCard, mj(map[string]any{"cardId": "c2"}))
	insertB("person:p1", KindAgentCard, map[string]any{"cardId": "c3"})
	e.append("agent:c1", KindAgentCard, `{"cardId":5}`)
	e.append("agent:c1", KindAgentCard, `null`)
	e.append("agent:", KindAgentCard, `{"CARDID":""}`)
	e.append("agent:c9", KindAgentCard, `{"CardId":"c9","cardid":"c9"}`)

	// Closure.
	const sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	e.append("person:p1", KindBoardCreated, mj(map[string]any{"boardId": "b", "name": "mlg", "tag": "mlg", "creator": map[string]any{"personId": "p1"}}))
	e.append("person:p1", "task.created", mj(map[string]any{"taskId": "t0", "by": "person:p2"}))
	e.append("person:p1", KindBoardMemberAdded, mj(map[string]any{"personId": "p2", "by": "p1", "role": "member"}))
	e.append("person:p1", KindBoardMemberRole, mj(map[string]any{"personId": "p2", "by": "p1", "role": "owner"}))
	e.append("person:p1", KindBoardMemberRemoved, mj(map[string]any{"personId": "p2", "by": "p1", "cut": map[string]any{"dev-b": 0}}))
	e.append("person:p1", "task.created", mj(map[string]any{"taskId": "t1", "title": "one", "by": "person:p1"}))
	e.append("agent:c1", KindAgentCard, mj(map[string]any{"cardId": "c1", "personId": "p1", "status": "active"}))
	closure := []struct {
		kind string
		row  map[string]any
	}{
		{"task.comment", map[string]any{"taskId": "private-task", "commentId": "m1", "by": "person:p1", "text": "hi"}},
		{"task.created", map[string]any{"taskId": "t9", "by": "dev-a/session-1"}},
		{"task.assigned", map[string]any{"taskId": "t1", "to": "user:dev-a", "by": "person:p1"}},
		{"task.joined", map[string]any{"taskId": "t1", "who": "agent:nobody", "by": "agent:nobody"}},
		{"blocker.option", map[string]any{"taskId": "t1", "blockerId": "b1", "answer": "x", "by": "person:p1"}},
		{"task.step", map[string]any{"taskId": "t1", "stepId": "s9", "by": "person:p1"}},
		{"task.comment", map[string]any{"taskId": "t1", "commentId": "m2", "by": "person:p1", "attachments": []map[string]any{{"sha256": sha}}}},
		{"task.comment", map[string]any{"taskId": "t1", "commentId": "m3", "msgId": "m3", "by": "person:p1"}},
		{"task.status", map[string]any{"taskId": "t1", "state": "blocked", "by": "person:p1", "blockedBy": map[string]any{"kind": "task", "ref": "private-task"}}},
		{"lease.released", map[string]any{"resource": "gpu/qwen", "holder": "agent:c1", "by": "agent:c1"}},
		{"task.parent", map[string]any{"taskId": "t1", "parent": "private-task", "by": "person:p1"}},
		{"task.created", map[string]any{"taskId": "t5", "parent": "private-task", "by": "person:p1"}},
		{"task.assigned", map[string]any{"taskId": "t1", "to": "agent:c1", "by": "person:p9"}},
		{"task.status", map[string]any{"taskId": "t1", "state": "completed", "by": "person:p1", "outcome": map[string]any{"kind": "edit", "task": "private-task"}}},
		{"lease.acquired", map[string]any{"resource": "gpu/x", "holder": "agent:c1", "stolenFrom": "agent:c1,dev-a/session-1"}},
		// more shapes than board_test.go covers
		{"task.status", map[string]any{"taskId": "t1", "blockedBy": map[string]any{"kind": "lease", "ref": "nope"}}},
		{"task.status", map[string]any{"taskId": "t1", "blockedBy": map[string]any{"kind": "agent", "ref": "agent:zz"}}},
		{"task.status", map[string]any{"taskId": "t1", "blockedBy": map[string]any{"kind": "person", "ref": "person:"}}},
		{"task.status", map[string]any{"taskId": "t1", "blockedBy": map[string]any{"kind": "other", "ref": "x"}}},
		{"task.status", map[string]any{"taskId": "t1", "blocker": map[string]any{"id": "b7", "on": map[string]any{"kind": "task", "ref": "zz"}}}},
		{"task.status", map[string]any{"taskId": "t1", "blocker": map[string]any{"id": "b7", "attachments": []map[string]any{{"sha256": "bb"}}}}},
		{"lease.stolen", map[string]any{"resource": "gpu/none", "holder": "agent:c1", "stolen": []map[string]any{{"resource": "gpu/none", "holder": "dev/x"}}}},
		{"lease.stolen", map[string]any{"resource": "gpu/none", "holder": "user"}},
		{"lease.refused", map[string]any{"resource": "gpu/a", "held": "gpu/b", "holder": "agent:c1"}},
		{"lease.acquired", map[string]any{"resource": "gpu/y", "holder": "agent:c1", "conflict": "person:zz"}},
		{"lease.acquired", map[string]any{"resource": "gpu/y", "heldBy": "agent:" + string(bytes.Repeat([]byte("a"), 129))}},
		{"task.comment", map[string]any{"taskId": "t1", "commentId": 5}},
		{"task.comment", map[string]any{"TASKID": "nope", "commentId": "m4"}},
		{"task.steps", map[string]any{"taskId": "t1", "steps": "x"}},
	}
	for _, tc := range closure {
		e.append("person:p1", tc.kind, mj(tc.row))
	}
	e.append("person:p1", "task.comment", `["not","an","object"]`)
	e.append("person:p1", "task.comment", `null`)
	e.append("person:p1", "task.comment", `"str"`)
	e.append("person:p1", "member.anything", `[1]`)
	e.append("person:p1", "task.steps", mj(map[string]any{"taskId": "t1", "steps": []map[string]any{{"id": "s1", "text": "go"}}, "by": "person:p1"}))
	e.append("person:p1", "task.step", mj(map[string]any{"taskId": "t1", "stepId": "s1", "by": "person:p1"}))
	e.append("person:p1", "task.assigned", mj(map[string]any{"taskId": "t1", "to": "agent:c1", "by": "person:p1"}))
	e.append("agent:c1", "task.status", mj(map[string]any{"taskId": "t1", "state": "blocked", "by": "agent:c1", "blocker": map[string]any{"id": "b1", "question": "which?"}}))
	e.append("person:p1", "blocker.option", mj(map[string]any{"taskId": "t1", "blockerId": "b1", "answer": "x", "by": "person:p1"}))
	e.append("irori", KindAttachOffered, mj(map[string]any{"sha256": sha, "size": 1, "name": "a.txt"}))
	e.append("person:p1", "task.comment", mj(map[string]any{"taskId": "t1", "commentId": "m2", "by": "person:p1", "attachments": []map[string]any{{"sha256": sha}}}))
	e.append("person:p1", "task.linked", mj(map[string]any{"taskId": "t1", "link": map[string]any{"kind": "commit", "ref": "abc1234"}, "by": "person:p1"}))
	e.append("agent:c1", "lease.acquired", mj(map[string]any{"resource": "gpu/qwen", "holder": "agent:c1"}))
	e.append("agent:c1", "lease.released", mj(map[string]any{"resource": "gpu/qwen", "holder": "agent:c1", "by": "agent:c1"}))
	e.append("agent:c1", "lease.stolen", mj(map[string]any{"resource": "gpu/qwen", "holder": "agent:c1", "stolen": []map[string]any{{"resource": "gpu/qwen", "holder": "person:p1"}}, "stolenFrom": " person:p2 , agent:c1"}))
	e.append("person:p1", "task.status", mj(map[string]any{"taskId": "t1", "blockedBy": map[string]any{"kind": "person", "ref": "person:p2"}, "outcome": map[string]any{"task": "t1"}}))
	e.append("person:p1", "task.comment", `{"taskId":"t1","commentId":"m5","by":null,"attachments":null}`)
	e.append("person:p1", "task.comment", `{"TaskId":"t1","taskid":"private-task","commentId":"m6"}`)

	// A forwarded row is stored even when it doesn't resolve yet.
	early := insertB("agent:c2", "task.comment", map[string]any{"taskId": "t2", "commentId": "m9", "by": "agent:c2", "text": "early"})
	e.unresolved(early)
	insertB("person:p2", "task.created", map[string]any{"taskId": "t2", "title": "two", "by": "person:p2"})
	e.unresolved(early)
	insertB("agent:c2", KindAgentCard, map[string]any{"cardId": "c2", "personId": "p2", "status": "active"})
	e.unresolved(early)
	e.unresolved(Event{Kind: "task.comment", Payload: json.RawMessage(`{"taskId":"t2","by":"agent:c3"}`)})
	e.unresolved(Event{Kind: "board.anything", Payload: json.RawMessage(`[`)})
	e.unresolved(Event{Kind: "task.comment", Payload: json.RawMessage(`{`)})
	// An uncertified device.
	e.insert(e.row("dev-x", 1, "person:p9", "task.created", `{"taskId":"t9"}`, true))
	e.snapshot()

	// The index is rebuilt at Open.
	e.reopen()
	e.append("agent:c2", "task.comment", mj(map[string]any{"taskId": "t2", "commentId": "m10", "by": "agent:c2", "text": "after reopen"}))
	e.append("person:p1", "task.comment", mj(map[string]any{"taskId": "private-task", "commentId": "m11", "by": "person:p1"}))
	e.snapshot()
	goldWrite(t, "board.json", map[string]any{"self": e.self, "ops": e.ops})
}

// ---------------------------------------------------------------- differential (not from export.sh)

// TestDiffOpenRustDB opens, with the Go store, a scratch copy the Rust
// store has written to (IRORI_RUST_DB, as device IRORI_RUST_DEVICE), checks
// every events row's RowHash against the list Rust wrote
// (IRORI_RUST_MANIFEST, "events \t device \t seq \t hash" lines), and
// appends one row: the Go build still opens, reads and extends a log Rust
// wrote (architecture.md §8 M2).
func TestDiffOpenRustDB(t *testing.T) {
	path, dev, manifest := os.Getenv("IRORI_RUST_DB"), os.Getenv("IRORI_RUST_DEVICE"), os.Getenv("IRORI_RUST_MANIFEST")
	if path == "" || dev == "" || manifest == "" {
		t.Skip("IRORI_RUST_DB / IRORI_RUST_DEVICE / IRORI_RUST_MANIFEST not set")
	}
	raw, err := os.ReadFile(manifest)
	if err != nil {
		t.Fatal(err)
	}
	want := map[string]string{}
	for _, line := range bytes.Split(bytes.TrimSpace(raw), []byte("\n")) {
		f := bytes.Split(line, []byte("\t"))
		if len(f) != 4 {
			t.Fatalf("bad manifest line")
		}
		want[string(f[1])+"\t"+string(f[2])] = string(f[3])
	}
	ctx := context.Background()
	s, err := Open(ctx, path, dev, hlc.New(nil))
	if err != nil {
		t.Fatalf("Go cannot open the log Rust wrote: %v", err)
	}
	defer s.Close()
	got := 0
	var after Cursor
	for {
		evs, err := s.List(ctx, Filter{After: after, Limit: MaxListLimit})
		if err != nil {
			t.Fatal(err)
		}
		for _, ev := range evs {
			k := fmt.Sprintf("%s\t%d", ev.DeviceID, ev.Seq)
			if want[k] != RowHash(ev) {
				t.Errorf("row %s/%d: RowHash differs from Rust's", ev.DeviceID, ev.Seq)
			}
			got++
		}
		if len(evs) < MaxListLimit {
			break
		}
		after = evs[len(evs)-1].Cursor()
	}
	if got != len(want) {
		t.Fatalf("Go reads %d rows, Rust listed %d", got, len(want))
	}
	if _, err := s.Append(ctx, "irori", KindAgentSeen, map[string]string{"by": "go"}); err != nil {
		t.Fatalf("Go append after Rust: %v", err)
	}
	t.Logf("Go opened the Rust-written log: %d rows match, one appended", got)
}

// TestDiffLiveRowHashes reads a scratch copy of a real irori.db (raw SQL,
// no Open: nothing is written) and writes one line per row:
//
//	table \t device_id \t seq \t RowHash [\t other_hash]
//
// so the Rust store can compare its RowHash of the same rows. Nothing
// else of a row is written.
func TestDiffLiveRowHashes(t *testing.T) {
	src, out := os.Getenv("IRORI_DIFF_DB"), os.Getenv("IRORI_DIFF_OUT")
	if src == "" || out == "" {
		t.Skip("IRORI_DIFF_DB / IRORI_DIFF_OUT not set")
	}
	// Its own scratch copy: closing may checkpoint the copy's WAL.
	sdb, err := sql.Open("sqlite", src)
	if err != nil {
		t.Fatal(err)
	}
	defer sdb.Close()
	var buf bytes.Buffer
	n := 0
	for _, table := range []string{"events", "local_events", "refused_events"} {
		evs, err := (&Store{impl: &db{sql: sdb}}).query(context.Background(),
			`SELECT device_id, seq, hlc, actor, kind, payload, sig FROM `+table+` ORDER BY device_id, seq`)
		if err != nil {
			if errors.Is(err, sql.ErrNoRows) {
				continue
			}
			t.Fatal(err)
		}
		for _, ev := range evs {
			fmt.Fprintf(&buf, "%s\t%s\t%d\t%s\n", table, ev.DeviceID, ev.Seq, RowHash(ev))
			n++
		}
	}
	rows, err := sdb.Query(`SELECT device_id, seq, hlc, actor, kind, payload, sig, other_hash FROM forks ORDER BY device_id, seq, other_hash`)
	if err != nil {
		t.Fatal(err)
	}
	for rows.Next() {
		var ev Event
		var h int64
		var payload, sig []byte
		var other string
		if err := rows.Scan(&ev.DeviceID, &ev.Seq, &h, &ev.Actor, &ev.Kind, &payload, &sig, &other); err != nil {
			t.Fatal(err)
		}
		ev.HLC, ev.Payload, ev.Sig = hlc.Timestamp(h), json.RawMessage(payload), sig
		fmt.Fprintf(&buf, "forks\t%s\t%d\t%s\t%s\n", ev.DeviceID, ev.Seq, RowHash(ev), other)
		n++
	}
	rows.Close()
	if err := os.WriteFile(out, buf.Bytes(), 0o600); err != nil {
		t.Fatal(err)
	}
	t.Logf("%d rows hashed", n)
}

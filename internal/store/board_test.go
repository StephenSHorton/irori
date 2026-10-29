package store

import (
	"context"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"fmt"
	"path/filepath"
	"sort"
	"sync"
	"testing"

	"github.com/StephenSHorton/irori/internal/hlc"
)

// devVerifier certifies a fixed set of devices by key and records every
// row the store asks it to Observe: the board Verifier hook (v0.5 §3)
// without the identity machinery.
type devVerifier struct {
	mu       sync.Mutex
	pubs     map[string]ed25519.PublicKey
	observed []string
	resets   int
}

func (v *devVerifier) VerifyRow(ev Event) error {
	v.mu.Lock()
	pub, ok := v.pubs[ev.DeviceID]
	v.mu.Unlock()
	if !ok {
		return fmt.Errorf("%w: %s", ErrNoCert, ev.DeviceID)
	}
	if !ed25519.Verify(pub, SigningBytes(ev), ev.Sig) {
		return ErrBadSig
	}
	return nil
}
func (v *devVerifier) Cut(string) (int64, bool) { return 0, false }
func (v *devVerifier) CutClaims() []CutClaim    { return nil }
func (v *devVerifier) Observe(ev Event) {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.observed = append(v.observed, ev.Kind)
}
func (v *devVerifier) Reset() {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.observed, v.resets = nil, v.resets+1
}

// boardEnv is a board log on dev-a, with dev-b's signer to forge
// replicated rows from.
type boardEnv struct {
	t      *testing.T
	path   string
	s      *Store
	ver    *devVerifier
	a, b   keySigner
	clockB *hlc.Clock
	seqB   int64
}

func newBoardEnv(t *testing.T) *boardEnv {
	t.Helper()
	e := &boardEnv{t: t, path: filepath.Join(t.TempDir(), "boards", "0123456789abcdef0123456789abcdef.db"),
		a: newSigner(t), b: newSigner(t), clockB: hlc.New(nil)}
	e.ver = &devVerifier{pubs: map[string]ed25519.PublicKey{
		"dev-a": e.a.key.Public().(ed25519.PublicKey), "dev-b": e.b.key.Public().(ed25519.PublicKey)}}
	e.open()
	return e
}

func (e *boardEnv) open() {
	e.t.Helper()
	s, err := OpenWith(context.Background(), e.path, "dev-a", hlc.New(nil), OpenOptions{Signer: e.a, Verifier: e.ver, Board: true})
	if err != nil {
		e.t.Fatal(err)
	}
	e.s = s
	e.t.Cleanup(func() { s.Close() })
}

// fromB is a row dev-b signed, with the next seq of dev-b's stream.
func (e *boardEnv) fromB(actor, kind string, payload any) Event {
	e.t.Helper()
	raw, err := json.Marshal(payload)
	if err != nil {
		e.t.Fatal(err)
	}
	ev := Event{DeviceID: "dev-b", Seq: e.seqB + 1, HLC: e.clockB.Now(), Actor: actor, Kind: kind, Payload: raw}
	ev.Sig, _ = e.b.SignRow(SigningBytes(ev))
	return ev
}

// insertB inserts a dev-b row; on success dev-b's stream moves on.
func (e *boardEnv) insertB(actor, kind string, payload any) (Event, error) {
	ev := e.fromB(actor, kind, payload)
	err := e.s.Insert(context.Background(), ev)
	if err == nil {
		e.seqB++
	}
	return ev, err
}

func (e *boardEnv) append(actor, kind string, payload any) error {
	_, err := e.s.Append(context.Background(), actor, kind, payload)
	return err
}

func (e *boardEnv) mustAppend(actor, kind string, payload any) {
	e.t.Helper()
	if err := e.append(actor, kind, payload); err != nil {
		e.t.Fatalf("append %s as %s: %v", kind, actor, err)
	}
}

// TestBoardRefusesKinds: every kind the registry keeps out of board logs,
// and a kind it doesn't know, is refused by Append and by Insert and
// counted (v0.5 §2.2). The private log still takes them.
func TestBoardRefusesKinds(t *testing.T) {
	e := newBoardEnv(t)
	private := openTest(t, t.TempDir(), "dev-a")
	var refused []string
	for kind, side := range kindSides {
		if side == Replicated && KindBoard(kind) == BoardRefuses {
			refused = append(refused, kind)
		}
	}
	sort.Strings(refused)
	refused = append(refused, "future.kind")
	for _, want := range []string{"message.sent", "capture.turn", "agent.seen", "policy.set", "person.device.added",
		"task.shared", "node.addr", "ask.posted", "board.joined", "receive.set", "board.sharedcopy", "board.cardof"} {
		found := false
		for _, k := range refused {
			found = found || k == want
		}
		if !found && !IsLocal(want) {
			t.Errorf("%s should be refused by a board log", want)
		}
	}
	for i, kind := range refused {
		t.Run(kind, func(t *testing.T) {
			if err := e.append("irori", kind, map[string]any{}); !errors.Is(err, ErrNotBoardKind) {
				t.Errorf("Append: err = %v, want ErrNotBoardKind", err)
			}
			if _, err := e.insertB("irori", kind, map[string]any{}); !errors.Is(err, ErrNotBoardKind) {
				t.Errorf("Insert: err = %v, want ErrNotBoardKind", err)
			}
			if got := e.s.BoardRefused().Kind; got != 2*(i+1) {
				t.Errorf("refused count = %d, want %d", got, 2*(i+1))
			}
			if _, err := private.Append(context.Background(), "irori", kind, map[string]any{}); err != nil {
				t.Errorf("the private log refused %s: %v", kind, err)
			}
		})
	}
	// Device-local kinds stay refused as local first.
	for _, kind := range []string{KindPolicySet, KindMemberJoined, KindBoardCardKey} {
		if err := e.append("irori", kind, map[string]any{}); !errors.Is(err, ErrLocalKind) {
			t.Errorf("Append %s: err = %v, want ErrLocalKind", kind, err)
		}
	}
	if got := all(t, e.s); len(got) != 0 {
		t.Fatalf("a refused row was stored: %+v", got)
	}
}

// TestBoardActors: board rows never carry "user" (§2.4): only irori,
// person:<id> and agent:<cardId>, in Append and Insert alike.
func TestBoardActors(t *testing.T) {
	e := newBoardEnv(t)
	e.mustAppend("agent:c1", KindAgentCard, map[string]any{"cardId": "c1", "name": "lead", "kind": "claude", "personId": "p1", "status": "active"})
	cases := []struct {
		actor string
		ok    bool
	}{
		{"irori", true},
		{"person:p1", true},
		{"agent:c1", true},
		{"user", false},
		{"user:dev-a", false},
		{"user:anything", false},
		{"api:dev-a/s1", false},
		{"agent:dev-a/s1", false}, // a private agent key names its device and session
		{"agent:", false},
		{"person:", false},
		{"person:p1/x", false},
		{"", false},
	}
	for _, tc := range cases {
		t.Run(tc.actor, func(t *testing.T) {
			row := map[string]any{"taskId": "t-" + tc.actor, "title": "x", "by": ""}
			err := e.append(tc.actor, "task.created", row)
			if tc.ok != (err == nil) || (!tc.ok && !errors.Is(err, ErrBoardActor)) {
				t.Errorf("Append as %q: err = %v, want ok=%v", tc.actor, err, tc.ok)
			}
			_, err = e.insertB(tc.actor, "task.created", row)
			if tc.ok != (err == nil) || (!tc.ok && !errors.Is(err, ErrBoardActor)) {
				t.Errorf("Insert as %q: err = %v, want ok=%v", tc.actor, err, tc.ok)
			}
		})
	}
	// An agent card is written as the agent it describes.
	if err := e.append("agent:c1", KindAgentCard, map[string]any{"cardId": "c2"}); !errors.Is(err, ErrBoardActor) {
		t.Errorf("a card for c2 written as c1: err = %v", err)
	}
	if _, err := e.insertB("person:p1", KindAgentCard, map[string]any{"cardId": "c3"}); !errors.Is(err, ErrBoardActor) {
		t.Errorf("a card written as a person: err = %v", err)
	}
}

// TestBoardClosure: Append refuses a row naming what the board doesn't
// hold, with the reason; Insert stores it, and Unresolved says so until
// what it names arrives (§2.3).
func TestBoardClosure(t *testing.T) {
	e := newBoardEnv(t)
	const sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	refused := []struct {
		name, kind string
		row        map[string]any
	}{
		{"a private task", "task.comment", map[string]any{"taskId": "private-task", "commentId": "m1", "by": "person:p1", "text": "hi"}},
		{"a private agent key", "task.created", map[string]any{"taskId": "t9", "by": "dev-a/session-1"}},
		{"a user key", "task.assigned", map[string]any{"taskId": "t1", "to": "user:dev-a", "by": "person:p1"}},
		{"an unknown card", "task.joined", map[string]any{"taskId": "t1", "who": "agent:nobody", "by": "agent:nobody"}},
		{"a blocker of another card", "blocker.option", map[string]any{"taskId": "t1", "blockerId": "b1", "answer": "x", "by": "person:p1"}},
		{"an unknown step", "task.step", map[string]any{"taskId": "t1", "stepId": "s9", "by": "person:p1"}},
		{"an unoffered attachment", "task.comment", map[string]any{"taskId": "t1", "commentId": "m2", "by": "person:p1",
			"attachments": []map[string]any{{"sha256": sha}}}},
		{"a private message", "task.comment", map[string]any{"taskId": "t1", "commentId": "m3", "msgId": "m3", "by": "person:p1"}},
		{"a private blocked-on task", "task.status", map[string]any{"taskId": "t1", "state": "blocked", "by": "person:p1",
			"blockedBy": map[string]any{"kind": "task", "ref": "private-task"}}},
		{"an unmirrored lease", "lease.released", map[string]any{"resource": "gpu/qwen", "holder": "agent:c1", "by": "agent:c1"}},
		{"a private parent", "task.parent", map[string]any{"taskId": "t1", "parent": "private-task", "by": "person:p1"}},
		{"a subtask of a private card", "task.created", map[string]any{"taskId": "t5", "parent": "private-task", "by": "person:p1"}},
		{"a person not on the board", "task.assigned", map[string]any{"taskId": "t1", "to": "agent:c1", "by": "person:p9"}},
		{"an outcome naming a private card", "task.status", map[string]any{"taskId": "t1", "state": "completed", "by": "person:p1",
			"outcome": map[string]any{"kind": "edit", "task": "private-task"}}},
		{"a lease stolen from a private key", "lease.acquired", map[string]any{"resource": "gpu/x", "holder": "agent:c1", "stolenFrom": "agent:c1,dev-a/session-1"}},
	}
	// board.* rows name persons bare and are never closure-checked; they
	// put their persons on the board for person: keys.
	e.mustAppend("person:p1", KindBoardCreated, map[string]any{"boardId": "b", "name": "mlg", "tag": "mlg", "creator": map[string]any{"personId": "p1"}})
	if err := e.append("person:p1", "task.created", map[string]any{"taskId": "t0", "by": "person:p2"}); !errors.Is(err, ErrOutsideBoard) {
		t.Fatalf("a person before their board.member.added: err = %v", err)
	}
	e.mustAppend("person:p1", KindBoardMemberAdded, map[string]any{"personId": "p2", "by": "p1", "role": "member"})
	e.mustAppend("person:p1", KindBoardMemberRole, map[string]any{"personId": "p2", "by": "p1", "role": "owner"})
	e.mustAppend("person:p1", KindBoardMemberRemoved, map[string]any{"personId": "p2", "by": "p1", "cut": map[string]any{"dev-b": 0}})
	e.mustAppend("person:p1", "task.created", map[string]any{"taskId": "t1", "title": "one", "by": "person:p1"})
	e.mustAppend("agent:c1", KindAgentCard, map[string]any{"cardId": "c1", "personId": "p1", "status": "active"})
	for _, tc := range refused {
		t.Run(tc.name, func(t *testing.T) {
			err := e.append("person:p1", tc.kind, tc.row)
			if !errors.Is(err, ErrOutsideBoard) {
				t.Fatalf("Append: err = %v, want ErrOutsideBoard", err)
			}
			t.Logf("reason: %v", err)
		})
	}
	if got := e.s.BoardRefused().Outside; got != len(refused)+1 { // and person:p2 before their add
		t.Errorf("outside count = %d, want %d", got, len(refused)+1)
	}
	// What is on the board may be named.
	e.mustAppend("person:p1", "task.steps", map[string]any{"taskId": "t1", "steps": []map[string]any{{"id": "s1", "text": "go"}}, "by": "person:p1"})
	e.mustAppend("person:p1", "task.step", map[string]any{"taskId": "t1", "stepId": "s1", "by": "person:p1"})
	e.mustAppend("person:p1", "task.assigned", map[string]any{"taskId": "t1", "to": "agent:c1", "by": "person:p1"})
	e.mustAppend("agent:c1", "task.status", map[string]any{"taskId": "t1", "state": "blocked", "by": "agent:c1",
		"blocker": map[string]any{"id": "b1", "question": "which?"}})
	e.mustAppend("person:p1", "blocker.option", map[string]any{"taskId": "t1", "blockerId": "b1", "answer": "x", "by": "person:p1"})
	e.mustAppend("irori", KindAttachOffered, map[string]any{"sha256": sha, "size": 1, "name": "a.txt"})
	e.mustAppend("person:p1", "task.comment", map[string]any{"taskId": "t1", "commentId": "m2", "by": "person:p1",
		"attachments": []map[string]any{{"sha256": sha}}})
	e.mustAppend("person:p1", "task.linked", map[string]any{"taskId": "t1", "link": map[string]any{"kind": "commit", "ref": "abc1234"}, "by": "person:p1"})
	e.mustAppend("agent:c1", "lease.acquired", map[string]any{"resource": "gpu/qwen", "holder": "agent:c1"})
	e.mustAppend("agent:c1", "lease.released", map[string]any{"resource": "gpu/qwen", "holder": "agent:c1", "by": "agent:c1"})

	// A forwarded row is stored even when it doesn't resolve yet.
	early, err := e.insertB("agent:c2", "task.comment", map[string]any{"taskId": "t2", "commentId": "m9", "by": "agent:c2", "text": "early"})
	if err != nil {
		t.Fatalf("Insert of an unresolved row: %v", err)
	}
	if err := e.s.Unresolved(early); !errors.Is(err, ErrOutsideBoard) {
		t.Fatalf("Unresolved before its task: %v", err)
	}
	if _, err := e.insertB("person:p2", "task.created", map[string]any{"taskId": "t2", "title": "two", "by": "person:p2"}); err != nil {
		t.Fatal(err)
	}
	if err := e.s.Unresolved(early); !errors.Is(err, ErrOutsideBoard) {
		t.Fatalf("Unresolved before its card: %v", err)
	}
	if _, err := e.insertB("agent:c2", KindAgentCard, map[string]any{"cardId": "c2", "personId": "p2", "status": "active"}); err != nil {
		t.Fatal(err)
	}
	if err := e.s.Unresolved(early); err != nil {
		t.Fatalf("Unresolved once everything arrived: %v", err)
	}

	// The index is rebuilt at Open.
	e.s.Close()
	e.open()
	e.mustAppend("agent:c2", "task.comment", map[string]any{"taskId": "t2", "commentId": "m10", "by": "agent:c2", "text": "after reopen"})
	if err := e.append("person:p1", "task.comment", map[string]any{"taskId": "private-task", "commentId": "m11", "by": "person:p1"}); !errors.Is(err, ErrOutsideBoard) {
		t.Fatalf("after reopen: err = %v", err)
	}
	// The private log never checks closure.
	private := openTest(t, t.TempDir(), "dev-a")
	if err := private.Unresolved(early); err != nil {
		t.Fatalf("private Unresolved: %v", err)
	}
	if private.IsBoard() || !e.s.IsBoard() {
		t.Fatal("IsBoard")
	}
}

// TestBoardVerifierHook: a board store's Verifier folds board.* and member.*
// rows, at Open and as they are written (§3), and refuses an uncertified
// device's rows like the private log's does.
func TestBoardVerifierHook(t *testing.T) {
	e := newBoardEnv(t)
	e.mustAppend("irori", KindMemberProfile, map[string]any{"personId": "p1", "name": "stephen"})
	e.mustAppend("irori", "task.created", map[string]any{"taskId": "t1", "by": "irori"})
	if _, err := e.insertB("irori", KindBoardRenamed, map[string]any{"name": "mlg", "by": "person:p2"}); err != nil {
		t.Fatal(err)
	}
	e.ver.mu.Lock()
	got := append([]string(nil), e.ver.observed...)
	e.ver.mu.Unlock()
	if want := []string{KindMemberProfile, KindBoardRenamed}; fmt.Sprint(got) != fmt.Sprint(want) {
		t.Fatalf("observed %v, want %v", got, want)
	}
	e.s.Close()
	e.ver.Reset()
	e.open()
	e.ver.mu.Lock()
	got = append([]string(nil), e.ver.observed...)
	e.ver.mu.Unlock()
	if len(got) != 2 {
		t.Fatalf("observed at Open %v, want the two identity rows", got)
	}
	// An uncertified device is refused.
	_, k, _ := ed25519.GenerateKey(nil)
	ev := Event{DeviceID: "dev-x", Seq: 1, HLC: hlc.New(nil).Now(), Actor: "person:p9", Kind: "task.created",
		Payload: json.RawMessage(`{"taskId":"t9"}`)}
	ev.Sig = ed25519.Sign(k, SigningBytes(ev))
	if err := e.s.Insert(context.Background(), ev); !errors.Is(err, ErrNoCert) {
		t.Fatalf("uncertified device: err = %v, want ErrNoCert", err)
	}
}

package main

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/pair"
)

// syncBuf (follow_test.go) takes joinLoop's output while the test reads it.

func pairOpts(t *testing.T, net *mesh.FakeNet) core.Options {
	return core.Options{DataDir: t.TempDir(), ClaudeDir: t.TempDir(), Version: "0.4.2-test", Headless: true,
		Keystore: identity.FileKeystore{}, Net: net}
}

func startCore(t *testing.T, o core.Options) *core.Core {
	t.Helper()
	c, err := core.Start(context.Background(), o)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	return c
}

func waitUntil(t *testing.T, what string, ok func() bool) {
	t.Helper()
	deadline := time.Now().Add(15 * time.Second)
	for time.Now().Before(deadline) {
		if ok() {
			return
		}
		time.Sleep(15 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for: %s", what)
}

// joinFlow runs `irori pair join`'s loop on the joining device with the
// lines piped in (no terminal), while the test plays the person at the
// adding device: it types the words the joining side printed.
func joinFlow(t *testing.T, side joinSide, joiner func() pair.Status, pc *core.Core, lines chan string) (string, error) {
	t.Helper()
	add, err := pc.Pairing().Add(identity.RoleDesktop)
	if err != nil {
		t.Fatal(err)
	}
	lines <- "0000 0000 0000 0000" // a wrong key first
	lines <- add.Fingerprint
	out := &syncBuf{}
	done := make(chan error, 1)
	go func() {
		done <- joinLoop(context.Background(), side, add.Code, "", lines, out, 20*time.Millisecond)
	}()
	waitUntil(t, "the words are printed", func() bool { return strings.Contains(out.String(), "Type these words") })
	words := strings.Join(joiner().Words, " ")
	if !strings.Contains(out.String(), words) {
		t.Fatalf("the words aren't printed: %q", out.String())
	}
	waitUntil(t, "the pc asks for them", func() bool { return pc.Pairing().Status().State == pair.StateConfirm })
	if err := pc.Pairing().Confirm(context.Background(), words); err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-done:
		return out.String(), err
	case <-time.After(15 * time.Second):
		t.Fatal("the join never finished")
	}
	return "", nil
}

// While Irori runs here, `irori pair join` drives that Core through the
// API instead of failing with "already running".
func TestPairJoinDrivesTheRunningCore(t *testing.T) {
	net := mesh.NewFakeNet()
	pc := startCore(t, pairOpts(t, net))
	mo := pairOpts(t, net)
	mac := startCore(t, mo)
	side, closeSide, err := openJoin(context.Background(), mo)
	if err != nil {
		t.Fatal(err)
	}
	defer closeSide()
	if _, ok := side.(apiJoin); !ok {
		t.Fatalf("side = %T, want the running Core's API", side)
	}
	out, err := joinFlow(t, side, mac.Pairing().Status, pc, make(chan string, 4))
	if err != nil {
		t.Fatalf("%v\n%s", err, out)
	}
	if !strings.Contains(out, "That isn't the key. Try once more") || !strings.Contains(out, "Paired.") {
		t.Fatalf("output:\n%s", out)
	}
	if mac.Person().ID != pc.Person().ID {
		t.Fatal("the mac didn't take the pc's person")
	}
}

// With no Irori running, it starts its own Core, as before.
func TestPairJoinStartsItsOwnCore(t *testing.T) {
	net := mesh.NewFakeNet()
	pc := startCore(t, pairOpts(t, net))
	mo := pairOpts(t, net)
	side, closeSide, err := openJoin(context.Background(), mo)
	if err != nil {
		t.Fatal(err)
	}
	defer closeSide()
	local, ok := side.(localJoin)
	if !ok {
		t.Fatalf("side = %T, want a Core of its own", side)
	}
	out, err := joinFlow(t, side, local.p.Status, pc, make(chan string, 4))
	if err != nil {
		t.Fatalf("%v\n%s", err, out)
	}
	if !strings.Contains(out, "Paired.") {
		t.Fatalf("output:\n%s", out)
	}
}

// Stdin ending before a key cancels the running Core's join.
func TestPairJoinCancelsWhenStdinEnds(t *testing.T) {
	net := mesh.NewFakeNet()
	pc := startCore(t, pairOpts(t, net))
	mo := pairOpts(t, net)
	mac := startCore(t, mo)
	side, closeSide, err := openJoin(context.Background(), mo)
	if err != nil {
		t.Fatal(err)
	}
	defer closeSide()
	add, err := pc.Pairing().Add(identity.RoleDesktop)
	if err != nil {
		t.Fatal(err)
	}
	lines := make(chan string)
	close(lines)
	err = joinLoop(context.Background(), side, add.Code, "", lines, &syncBuf{}, 20*time.Millisecond)
	if err == nil || err.Error() != "no key typed" {
		t.Fatalf("err = %v", err)
	}
	if st := mac.Pairing().Status(); st.State != pair.StateCanceled {
		t.Fatalf("the mac's join = %s", st.State)
	}
}

// The prompts take the code and the server address from piped lines.
func TestAskJoin(t *testing.T) {
	lines := make(chan string, 2)
	lines <- "abcd-efgh-jkmn-pqrs"
	lines <- " 203.0.113.7:24817 "
	code, server, err := askJoin(lines, &syncBuf{})
	if err != nil || code != "abcd-efgh-jkmn-pqrs" || server != "203.0.113.7:24817" {
		t.Fatalf("askJoin = %q %q %v", code, server, err)
	}
	lines = make(chan string, 2)
	lines <- "abcd"
	lines <- "no port"
	if _, _, err := askJoin(lines, &syncBuf{}); err == nil {
		t.Fatal("a bad server address passed")
	}
	empty := make(chan string)
	close(empty)
	if _, _, err := askJoin(empty, &syncBuf{}); err == nil {
		t.Fatal("no code passed")
	}
}

// fakeJoin is a joinSide that plays back statuses, for the ways out of
// joinLoop that a real pairing can't stage on cue.
type fakeJoin struct {
	first   pair.Status
	sts     []pair.Status // Status returns these in turn, then the last
	keyErr  error
	cancels int
}

func (f *fakeJoin) Join(context.Context, string, string) (pair.Status, error) { return f.first, nil }
func (f *fakeJoin) Status(context.Context) (pair.Status, error) {
	st := f.sts[0]
	if len(f.sts) > 1 {
		f.sts = f.sts[1:]
	}
	return st, nil
}
func (f *fakeJoin) Key(context.Context, string) error { return f.keyErr }
func (f *fakeJoin) Cancel()                           { f.cancels++ }

// Any way out of an open join but "Paired." closes it, so the running
// Core isn't left holding it; a closed one isn't canceled again.
func TestPairJoinCancelsOnEveryWayOut(t *testing.T) {
	exp := time.Now().Add(time.Minute)
	words := pair.Status{Mode: pair.ModeJoin, State: pair.StateWords, Expires: exp, Words: []string{"a", "b"}}
	// A key the API refused for another reason than a wrong key.
	f := &fakeJoin{first: pair.Status{Mode: pair.ModeJoin, State: pair.StateWaiting, Expires: exp},
		sts: []pair.Status{words}, keyErr: errors.New("boom")}
	lines := make(chan string, 1)
	lines <- "1111 2222 3333 4444"
	if err := joinLoop(context.Background(), f, "code", "", lines, &syncBuf{}, time.Millisecond); err == nil || f.cancels != 1 {
		t.Fatalf("a refused key: err %v, cancels %d", err, f.cancels)
	}
	// The pairing failed on its own: nothing left to cancel.
	failed := pair.Status{Mode: pair.ModeJoin, State: pair.StateFailed, Expires: exp, Error: "nope"}
	f = &fakeJoin{first: words, sts: []pair.Status{failed}}
	if err := joinLoop(context.Background(), f, "code", "", nil, &syncBuf{}, time.Millisecond); err == nil || err.Error() != "nope" || f.cancels != 0 {
		t.Fatalf("a failed pairing: err %v, cancels %d", err, f.cancels)
	}
	// Paired: nothing to cancel.
	f = &fakeJoin{first: words, sts: []pair.Status{{Mode: pair.ModeJoin, State: pair.StateDone, Expires: exp}}}
	if err := joinLoop(context.Background(), f, "code", "", nil, &syncBuf{}, time.Millisecond); err != nil || f.cancels != 0 {
		t.Fatalf("paired: err %v, cancels %d", err, f.cancels)
	}
}

// The running Core's status is shared with the app: once this join ended,
// an Add or a newer Join there is not this command's pairing, so it
// neither follows it to "Paired." nor cancels it.
func TestPairJoinFollowsOnlyItsOwnJoin(t *testing.T) {
	exp := time.Now().Add(time.Minute)
	mine := pair.Status{Mode: pair.ModeJoin, State: pair.StateWaiting, Expires: exp}
	for name, other := range map[string]pair.Status{
		"an add":       {Mode: pair.ModeAdd, State: pair.StateDone},
		"a newer join": {Mode: pair.ModeJoin, State: pair.StateDone, Expires: exp.Add(time.Second)},
	} {
		out := &syncBuf{}
		f := &fakeJoin{first: mine, sts: []pair.Status{mine, other}}
		err := joinLoop(context.Background(), f, "code", "", nil, out, time.Millisecond)
		if err == nil || err.Error() != "the pairing closed" || strings.Contains(out.String(), "Paired.") || f.cancels != 0 {
			t.Fatalf("%s: err %v, cancels %d, output %q", name, err, f.cancels, out.String())
		}
	}
}

// An unreadable irori.json doesn't stop `irori pair join`: with no Irori
// answering it starts its own Core, as before the API path.
func TestPairJoinPastAnUnreadableLockfile(t *testing.T) {
	mo := pairOpts(t, mesh.NewFakeNet())
	if err := os.WriteFile(filepath.Join(mo.DataDir, discovery.FileName), []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}
	side, closeSide, err := openJoin(context.Background(), mo)
	if err != nil {
		t.Fatal(err)
	}
	defer closeSide()
	if _, ok := side.(localJoin); !ok {
		t.Fatalf("side = %T, want a Core of its own", side)
	}
}

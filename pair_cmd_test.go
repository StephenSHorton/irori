package main

import (
	"context"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/core"
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

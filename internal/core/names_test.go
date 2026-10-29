package core

import (
	"context"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/api"
	"github.com/StephenSHorton/irori/internal/attach"
	"github.com/StephenSHorton/irori/internal/store"

	"github.com/StephenSHorton/irori/internal/names"
	"github.com/StephenSHorton/irori/internal/presence"
)

func testNamer(t *testing.T, self string, cards *[]presence.AgentCard) *namer {
	t.Helper()
	n := newNamer(t.TempDir(), self)
	n.mesh = func() []presence.AgentCard { return *cards }
	return n
}

func card(dev, sid, name, source string, started time.Time) presence.AgentCard {
	return presence.AgentCard{Key: dev + "/" + sid, DeviceID: dev, SessionID: sid, Agent: "claude",
		Name: name, NameSource: source, Status: "idle", StartedAt: started}
}

func TestNamerAssignsOnceAndKeepsItAcrossRestarts(t *testing.T) {
	var cards []presence.AgentCard
	dir := t.TempDir()
	n := newNamer(dir, "pc")
	n.mesh = func() []presence.AgentCard { return cards }
	got := n.Assign("s1")
	if got != names.Pick("s1", nil) {
		t.Fatalf("first name %q, want the seed's word %q", got, names.Pick("s1", nil))
	}
	if again := n.Assign("s1"); again != got {
		t.Fatalf("assigned twice: %q then %q", got, again)
	}
	if n.Assigned("s2") != "" {
		t.Fatal("Assigned gave a name to a session never named")
	}
	re := newNamer(dir, "pc")
	if re.Assigned("s1") != got {
		t.Fatalf("after a restart s1 is %q, want %q", re.Assigned("s1"), got)
	}
}

func TestNamerSkipsNamesLiveAgentsHave(t *testing.T) {
	first := names.Pick("s1", nil)
	cards := []presence.AgentCard{card("mac", "x", "Heron", "user", time.Time{})}
	cards[0].Name = first // another device's agent already has it, in any case
	n := testNamer(t, "pc", &cards)
	if got := n.Assign("s1"); names.Fold(got) == names.Fold(first) {
		t.Fatalf("picked %q, which a live agent has", got)
	}
	// A gone agent's name is free.
	cards[0].Status = "gone"
	if got := n.Assign("s9"); got == "" {
		t.Fatal("no name")
	}
}

func TestNamerSkipsNamesOtherLocalSessionsHold(t *testing.T) {
	var cards []presence.AgentCard
	n := testNamer(t, "pc", &cards)
	n.Assign("s1")
	// Give s1 the word s2's id hashes to, so s2 must probe past it.
	n.mu.Lock()
	n.m["s1"] = nameEntry{Name: names.Pick("s2", nil), At: n.now()}
	n.mu.Unlock()
	if got := n.Assign("s2"); got == names.Pick("s2", nil) {
		t.Fatalf("s2 got %q, which s1 holds", got)
	}
}

func TestSettleMovesTheLaterOfTwoIroriNamesOnBothDevices(t *testing.T) {
	t0 := time.Date(2026, 10, 28, 12, 0, 0, 0, time.UTC)
	// Two devices named their sessions while apart and both picked heron.
	pcCard := card("pc", "s1", "heron", presence.SourceIrori, t0)
	macCard := card("mac", "s2", "heron", presence.SourceIrori, t0.Add(time.Minute))
	cards := []presence.AgentCard{pcCard, macCard}

	pc := testNamer(t, "pc", &cards)
	pc.m["s1"] = nameEntry{Name: "heron", At: t0}
	mac := testNamer(t, "mac", &cards)
	mac.m["s2"] = nameEntry{Name: "heron", At: t0}

	if pc.Settle() {
		t.Fatal("the PC's session started first and should keep heron")
	}
	if !mac.Settle() {
		t.Fatal("the Mac's later session should move")
	}
	if got := mac.Assigned("s2"); got == "heron" || got == "" {
		t.Fatalf("the Mac's session is now %q", got)
	}
	if pc.Assigned("s1") != "heron" {
		t.Fatal("the PC's name moved")
	}
	// Claude keeps the title the mod set ("heron"); it must still read as
	// Irori's, so presence moves the card to the new name.
	if !mac.Gave("s2", "Heron") || !mac.Gave("s2", mac.Assigned("s2")) || mac.Gave("s2", "api work") {
		t.Fatal("the Mac's namer forgot which names it gave")
	}
}

func TestNamesOfLongIdleSessionsAreFreeForNewOnes(t *testing.T) {
	var cards []presence.AgentCard
	n := testNamer(t, "pc", &cards)
	now := time.Date(2026, 10, 28, 12, 0, 0, 0, time.UTC)
	n.now = func() time.Time { return now }
	want := names.Pick("new", nil)
	n.m["idle"] = nameEntry{Name: want, At: now.Add(-namesBusy - time.Minute)}
	if got := n.Assign("new"); got != want {
		t.Fatalf("got %q; a session idle that long shouldn't hold %q", got, want)
	}
	n.m["busy"] = nameEntry{Name: names.Pick("new2", nil), At: now}
	if got := n.Assign("new2"); got == names.Pick("new2", nil) {
		t.Fatalf("got %q, which a session in use holds", got)
	}
}

func TestSettleTieGoesToTheLowerKey(t *testing.T) {
	t0 := time.Date(2026, 10, 28, 12, 0, 0, 0, time.UTC)
	cards := []presence.AgentCard{
		card("aaa", "s1", "heron", presence.SourceIrori, t0),
		card("zzz", "s2", "heron", presence.SourceIrori, t0),
	}
	low := testNamer(t, "aaa", &cards)
	low.m["s1"] = nameEntry{Name: "heron", At: t0}
	high := testNamer(t, "zzz", &cards)
	high.m["s2"] = nameEntry{Name: "heron", At: t0}
	if low.Settle() || !high.Settle() {
		t.Fatal("on equal starts the lower key keeps the name")
	}
}

func TestSettleAChosenNameAlwaysKeepsIt(t *testing.T) {
	t0 := time.Date(2026, 10, 28, 12, 0, 0, 0, time.UTC)
	cards := []presence.AgentCard{
		card("pc", "s1", "heron", presence.SourceIrori, t0),
		card("mac", "s2", "Heron", presence.SourceUser, t0.Add(time.Hour)), // renamed later, by hand
	}
	pc := testNamer(t, "pc", &cards)
	pc.m["s1"] = nameEntry{Name: "heron", At: t0}
	if !pc.Settle() {
		t.Fatal("an Irori name must give way to a name someone chose")
	}
	if pc.Assigned("s1") == "heron" {
		t.Fatal("still heron")
	}
}

func TestSettleLeavesChosenNamesAlone(t *testing.T) {
	t0 := time.Date(2026, 10, 28, 12, 0, 0, 0, time.UTC)
	cards := []presence.AgentCard{
		card("pc", "s1", "api", presence.SourceUser, t0.Add(time.Hour)),
		card("mac", "s2", "api", presence.SourceUser, t0),
	}
	pc := testNamer(t, "pc", &cards)
	pc.m["s1"] = nameEntry{Name: "heron", At: t0}
	if pc.Settle() {
		t.Fatal("two chosen names are the person's to sort out, not Irori's")
	}
}

func TestSettleForgetsLongGoneSessions(t *testing.T) {
	var cards []presence.AgentCard
	n := testNamer(t, "pc", &cards)
	now := time.Date(2026, 10, 28, 12, 0, 0, 0, time.UTC)
	n.now = func() time.Time { return now }
	n.m["old"] = nameEntry{Name: "heron", At: now.Add(-namesKeep - time.Hour)}
	n.m["recent"] = nameEntry{Name: "otter", At: now.Add(-time.Hour)}
	n.Settle()
	if n.Assigned("old") != "" || n.Assigned("recent") != "otter" {
		t.Fatalf("old=%q recent=%q", n.Assigned("old"), n.Assigned("recent"))
	}
}

// An unnamed session gets an Irori name on its card and in the attach hello;
// renaming it in Claude reaches the mod as a name event and the log as an
// agent.status row with the new source (card #21).
func TestUnnamedSessionGetsANameAndARenameWins(t *testing.T) {
	old := api.Keepalive
	api.Keepalive = 50 * time.Millisecond
	t.Cleanup(func() { api.Keepalive = old })
	r := newRig(t)
	sessions := filepath.Join(r.core.opts.ClaudeDir, "sessions")
	cwd := t.TempDir()
	writeSession(t, sessions, "900001.json", sessA, "", cwd)
	waitCard(t, r, func(c presence.AgentCard) bool { return c.NameSource == presence.SourceIrori })

	s, hello := r.attach(sessA)
	defer s.cancel()
	if hello.NameSource != presence.SourceIrori || hello.Name != r.core.names.Assigned(sessA) || hello.Name == "" {
		t.Fatalf("hello name %q (%q), assigned %q", hello.Name, hello.NameSource, r.core.names.Assigned(sessA))
	}

	writeSession(t, sessions, "900001.json", sessA, "Bob", cwd)
	for {
		ev := s.next(attach.TypeName)
		if ev.Name == "Bob" {
			if ev.NameSource != presence.SourceUser {
				t.Fatalf("rename came as %q", ev.NameSource)
			}
			break
		}
	}
	deadline := time.Now().Add(5 * time.Second)
	for {
		evs, err := r.core.store.List(context.Background(), store.Filter{Kinds: []string{store.KindAgentStatus}, Limit: 500})
		if err != nil {
			t.Fatal(err)
		}
		for _, ev := range evs {
			if strings.Contains(string(ev.Payload), `"name":"Bob"`) && strings.Contains(string(ev.Payload), `"nameSource":"user"`) {
				return
			}
		}
		if time.Now().After(deadline) {
			t.Fatal("no agent.status row carried the rename and its source")
		}
		time.Sleep(50 * time.Millisecond)
	}
}

func waitCard(t *testing.T, r *rig, ok func(presence.AgentCard) bool) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for {
		for _, c := range r.core.Agents() {
			if c.Key == r.keyA && ok(c) {
				return
			}
		}
		if time.Now().After(deadline) {
			t.Fatalf("card never matched: %+v", r.core.Agents())
		}
		time.Sleep(50 * time.Millisecond)
	}
}

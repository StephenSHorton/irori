package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/presence"
)

func cardsBoard(t *testing.T) (*BoardService, *coord.Coord, *[]routed) {
	t.Helper()
	b, c, got := testBoard(t, nil)
	b.agents = func() []presence.AgentCard {
		return []presence.AgentCard{{Key: agentA, Name: "ww-release", Status: "busy"}, {Key: agentB, Name: "ww-multiplayer", Status: "idle"}}
	}
	b.prs = func(repo string) []ledger.Entry {
		if repo != "acme/app" {
			return nil
		}
		return []ledger.Entry{
			{Key: ledger.Key("acme/app", 57), Repo: "acme/app", Number: 57, Session: agentA, Role: ledger.Author},
			{Key: ledger.Key("acme/app", 57), Repo: "acme/app", Number: 57, Session: agentB, Role: ledger.Reviewer},
			{Key: ledger.Key("acme/app", 58), Repo: "acme/app", Number: 58, Session: agentB, Role: ledger.Author},
		}
	}
	return b, c, got
}

func TestCardsGrid(t *testing.T) {
	ctx := context.Background()
	b, c, _ := cardsBoard(t)
	if got := b.Cards(); got == nil || len(got) != 0 {
		t.Fatalf("empty grid %#v", got)
	}
	quiet := heldTask(t, c)
	blocked := heldTask(t, c)
	if _, err := c.Watch(ctx, agentB, blocked.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := c.SetSteps(ctx, agentA, blocked.ID, []coord.Step{{Text: "a", Done: true}, {Text: "b"}}); err != nil {
		t.Fatal(err)
	}
	if _, _, err := c.Raise(ctx, agentA, blocked.ID, coord.NewBlocker{Question: "Ship today?"}); err != nil {
		t.Fatal(err)
	}
	done := heldTask(t, c)
	if _, err := c.SetTaskState(ctx, agentA, done.ID, coord.Completed, ""); err != nil {
		t.Fatal(err)
	}
	got := b.Cards()
	if len(got) != 2 || got[0].ID != blocked.ID || got[1].ID != quiet.ID {
		t.Fatalf("grid %+v", got)
	}
	g := got[0]
	if g.Progress != (CardProgress{1, 2}) || g.OpenBlockers != 1 || g.BlockerQuestion != "Ship today?" ||
		g.Scope != ScopeMine || g.ScopeLabel != "My devices" || g.State != coord.Blocked {
		t.Fatalf("card %+v", g)
	}
	if len(g.Working) != 1 || g.Working[0] != (AgentRef{Key: agentA, Name: "ww-release", Status: "busy", Live: true, Where: "This PC"}) {
		t.Fatalf("working %+v", g.Working)
	}
	if len(g.Watching) != 1 || g.Watching[0].Name != "ww-multiplayer" || !g.Watching[0].Live {
		t.Fatalf("watching %+v", g.Watching)
	}
	if q := got[1]; q.Progress.Total != 0 || q.OpenBlockers != 0 || q.Watching == nil {
		t.Fatalf("quiet card %+v", q)
	}
	// Lists are never null for the webview.
	raw, _ := json.Marshal(got[1])
	if strings.Contains(string(raw), "null") {
		t.Fatalf("null in %s", raw)
	}
}

func TestCardDetail(t *testing.T) {
	ctx := context.Background()
	b, c, _ := cardsBoard(t)
	tk := heldTask(t, c)
	if _, err := c.AddLink(ctx, agentA, tk.ID, "https://github.com/acme/app/pull/57"); err != nil {
		t.Fatal(err)
	}
	if _, err := c.AddLink(ctx, agentA, tk.ID, "https://linear.app/acme/issue/ABC-1"); err != nil {
		t.Fatal(err)
	}
	if _, _, err := c.Raise(ctx, agentA, tk.ID, coord.NewBlocker{Question: "first?"}); err != nil {
		t.Fatal(err)
	}
	if _, _, err := c.Raise(ctx, agentA, tk.ID, coord.NewBlocker{Question: "Which adapter?", Context: "Radmin is closed.",
		Options: []coord.Option{{Answer: "Wintun", Why: "signed"}, {Answer: "TAP", Recommended: true}}}); err != nil {
		t.Fatal(err)
	}
	if _, _, err := c.Answer(ctx, "user:dev-a", tk.ID, "b1", coord.BlockerAnswer{Text: "yes"}); err != nil {
		t.Fatal(err)
	}
	d, err := b.Card(tk.ID)
	if err != nil {
		t.Fatal(err)
	}
	if len(d.Blockers) != 2 || d.Blockers[0].ID != "b2" || d.Blockers[0].Resolved != nil || d.Blockers[1].ID != "b1" ||
		d.Blockers[1].Resolved == nil || d.Blockers[1].Resolved.By.Name != "You" || d.Blockers[1].Resolved.Answer != "yes" {
		t.Fatalf("blockers %+v", d.Blockers)
	}
	if o := d.Blockers[0].Options; len(o) != 2 || o[0].By.Name != "ww-release" || o[0].Why != "signed" || o[0].Recommended ||
		!o[1].Recommended || d.Blockers[0].Context != "Radmin is closed." {
		t.Fatalf("options %+v", o)
	}
	if len(d.PRs) != 1 || d.PRs[0].Number != 57 || d.PRs[0].Label != "GitHub #57" ||
		len(d.PRs[0].OpenedBy) != 1 || d.PRs[0].OpenedBy[0].Name != "ww-release" ||
		len(d.PRs[0].ReviewedBy) != 1 || d.PRs[0].ReviewedBy[0].Name != "ww-multiplayer" {
		t.Fatalf("prs %+v", d.PRs)
	}
	if len(d.Links) != 2 || len(d.Recent) == 0 || !strings.HasPrefix(d.Recent[0].Text, "You answered “first?”") {
		t.Fatalf("links %+v recent %+v", d.Links, d.Recent)
	}
	if _, err := b.Card("nope"); err == nil {
		t.Fatal("unknown card opened")
	}
	// Capture off: the PR still shows, with nobody known.
	b.prs = nil
	if d, _ := b.Card(tk.ID); len(d.PRs) != 1 || d.PRs[0].OpenedBy == nil {
		t.Fatalf("no ledger %+v", d.PRs)
	}
}

func TestCardAnswer(t *testing.T) {
	cases := []struct {
		name      string
		option    int
		text      string
		wake      bool
		wantErr   bool
		wantAns   string
		wantMode  string
		wantState coord.TaskState
	}{
		{"pick and wake", 1, "", true, false, "TAP", "wake", coord.Working},
		{"own words, next turn", -1, " both ", false, false, "both", "next_turn", coord.Working},
		{"bad index", 5, "", true, true, "", "", ""},
		{"below -1", -2, "x", true, true, "", "", ""},
		{"no words", -1, " ", true, true, "", "", ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			ctx := context.Background()
			b, c, got := cardsBoard(t)
			tk := heldTask(t, c)
			if _, _, err := c.Raise(ctx, agentA, tk.ID, coord.NewBlocker{Question: "Which adapter?",
				Options: []coord.Option{{Answer: "Wintun"}, {Answer: "TAP"}}}); err != nil {
				t.Fatal(err)
			}
			res, err := b.Answer(tk.ID, "b1", tc.option, tc.text, tc.wake)
			if tc.wantErr {
				if err == nil {
					t.Fatal("no error")
				}
				if len(*got) != 0 {
					t.Fatalf("routed %+v", *got)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if res.Card.State != tc.wantState || res.Card.OpenBlockers != 0 || res.Card.Blockers[0].Resolved.Answer != tc.wantAns {
				t.Fatalf("card %+v", res.Card)
			}
			if res.Told.Mode != tc.wantMode || len(res.Told.To) != 1 || res.Told.To[0] != agentA {
				t.Fatalf("told %+v", res.Told)
			}
			if len(*got) != 1 || (*got)[0].n.Kind != coord.NoticeAnswered || (*got)[0].from.Key != "user:dev-a" || (*got)[0].n.Wake != tc.wake {
				t.Fatalf("routed %+v", *got)
			}
		})
	}
}

// The window re-reads on board:changed; card changes come through it.
func TestCardChangesReachThePump(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	_, c, _ := cardsBoard(t)
	tk := heldTask(t, c)
	ch := c.Subscribe(ctx)
	if _, err := c.SetSteps(ctx, agentA, tk.ID, []coord.Step{{Text: "a"}}); err != nil {
		t.Fatal(err)
	}
	select {
	case ch := <-ch:
		if ch.Kind != coord.TaskChanged || ch.TaskID != tk.ID {
			t.Fatalf("change %+v", ch)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("no change")
	}
}

// A card set to input_required asks the person its note: it shows as one
// open item (sorted first, counted as waiting on a decision), and answering
// it comments to the holder (a wake) and puts the card back to working.
func TestCardInputRequired(t *testing.T) {
	ctx := context.Background()
	b, c, got := cardsBoard(t)
	heldTask(t, c)
	tk := heldTask(t, c)
	if _, err := c.SetTaskState(ctx, agentA, tk.ID, coord.InputRequired, "Which DB should I use?"); err != nil {
		t.Fatal(err)
	}
	grid := b.Cards()
	if len(grid) != 2 || grid[0].ID != tk.ID || grid[0].OpenBlockers != 1 || grid[0].BlockerQuestion != "Which DB should I use?" {
		t.Fatalf("grid %+v", grid)
	}
	d, err := b.Card(tk.ID)
	if err != nil {
		t.Fatal(err)
	}
	if len(d.Blockers) != 1 || d.Blockers[0].ID != NoteItem || d.Blockers[0].Question != "Which DB should I use?" ||
		d.Blockers[0].RaisedBy.Key != agentA || d.Blockers[0].Resolved != nil {
		t.Fatalf("detail blockers %+v", d.Blockers)
	}
	if _, err := b.Answer(tk.ID, NoteItem, 0, "", true); err == nil {
		t.Fatal("picked an option the note doesn't have")
	}
	res, err := b.Answer(tk.ID, NoteItem, -1, "postgres", true)
	if err != nil {
		t.Fatal(err)
	}
	if res.Card.State != coord.Working || res.Card.OpenBlockers != 0 || res.Told.Mode != "wake" ||
		len(res.Told.To) != 1 || res.Told.To[0] != agentA {
		t.Fatalf("after answer %+v told %+v", res.Card.CardSummary, res.Told)
	}
	if len(res.Card.Comments) != 1 || res.Card.Comments[0].Text != "postgres" {
		t.Fatalf("comments %+v", res.Card.Comments)
	}
	if len(*got) != 1 || !strings.Contains((*got)[0].n.Text, "postgres") || !(*got)[0].n.Wake {
		t.Fatalf("routed %+v", *got)
	}
	if _, err := b.Answer(tk.ID, NoteItem, -1, "again", true); err == nil {
		t.Fatal("answered a note that no longer waits")
	}
}

// Faces from another device name it, and get the device chip only once the
// person has a second (unrevoked) device; an offline device's agents show
// greyed, and a contested lease marks its holder (v0.4 §9).
func TestCardFacesAcrossDevices(t *testing.T) {
	b, c, _ := cardsBoard(t)
	const mac = "dev-b/agent-c"
	b.self = "dev-a"
	b.agents = func() []presence.AgentCard {
		return []presence.AgentCard{
			{Key: agentA, DeviceID: "dev-a", Name: "ww-release", Status: "busy"},
			{Key: mac, DeviceID: "dev-b", Name: "mac-agent", Status: "idle", DeviceState: "offline"},
		}
	}
	devs := []identity.DeviceState{
		{Cert: identity.Cert{DeviceID: "dev-a", Label: "PC"}},
		{Cert: identity.Cert{DeviceID: "dev-b", Label: "Mac"}, Revoked: true},
	}
	b.devices = func() []identity.DeviceState { return devs }
	b.leases = func() []coord.Lease {
		return []coord.Lease{{Resource: "port:8799", Holder: mac, Conflict: agentA}, {Resource: "gpu", Holder: agentB}}
	}

	// One live device: names, but no chips.
	p := b.people()
	if r := p.ref(mac); r.Where != "Mac" || r.Device != "" || !r.Offline || r.Conflict != "port:8799" || !r.Live {
		t.Fatalf("mac face with one device %+v", r)
	}
	devs[1].Revoked = false
	p = b.people()
	if r := p.ref(mac); r.Where != "Mac" || r.Device != "Mac" || !r.Offline {
		t.Fatalf("mac face %+v", r)
	}
	if r := p.ref(agentA); r.Where != "This PC" || r.Device != "" || r.Offline || r.Conflict != "" {
		t.Fatalf("this device's face %+v", r)
	}
	// An agent the roster no longer has keeps its device from its key.
	if r := p.ref("dev-b/old"); r.Status != "gone" || r.Device != "Mac" {
		t.Fatalf("gone mac face %+v", r)
	}
	// A key with no device part gets no chip.
	if r := p.ref("api:composer"); r.Where != "This PC" || r.Device != "" {
		t.Fatalf("api face %+v", r)
	}
	// Cards carry the refs through.
	tk := heldTask(t, c)
	if g := b.Cards(); len(g) != 1 || g[0].ID != tk.ID || g[0].Working[0].Where != "This PC" {
		t.Fatalf("grid %+v", g)
	}
}

// The tray's Needs-a-decision count is the cards with an open blocker.
func TestDecideCount(t *testing.T) {
	ctx := context.Background()
	b, c, _ := cardsBoard(t)
	heldTask(t, c)
	if n := b.decideCount(); n != 0 {
		t.Fatalf("decide = %d", n)
	}
	tk := heldTask(t, c)
	if _, _, err := c.Raise(ctx, agentA, tk.ID, coord.NewBlocker{Question: "Ship?"}); err != nil {
		t.Fatal(err)
	}
	if n := b.decideCount(); n != 1 {
		t.Fatalf("decide = %d", n)
	}
}

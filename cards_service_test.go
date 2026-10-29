package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
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
		g.Scope != ScopeMine || g.ScopeLabel != "My devices" || g.State != coord.Blocked || g.Short != "2" || g.ShortWas != "" {
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

// A card's holder line: who holds it and whether that agent is active,
// idle or gone, and "waiting" for an assignment whose notice never reached
// its agent.
func TestCardHolderLine(t *testing.T) {
	ctx := context.Background()
	b, c, _ := cardsBoard(t)
	const gone = "dev-a/agent-gone"
	var open []inbox.Delivery
	b.deliveries = func() []inbox.Delivery { return open }
	notice := func(to, ref string, st inbox.State) inbox.Delivery {
		return inbox.Delivery{To: to, State: st, Message: inbox.Message{Ref: ref, Notice: true, Reason: string(coord.NoticeAssigned)}}
	}
	later := func(to, ref string, kind coord.NoticeKind) inbox.Delivery {
		return inbox.Delivery{To: to, State: inbox.Queued, Message: inbox.Message{Ref: ref, Notice: true, Reason: string(kind)}}
	}
	nobody, err := c.CreateTask(ctx, agentA, coord.NewTask{Title: "nobody"})
	if err != nil {
		t.Fatal(err)
	}
	active := heldTask(t, c) // agentA: busy
	idle := heldTask(t, c)
	if _, _, err := c.Assign(ctx, agentA, idle.ID, agentB); err != nil {
		t.Fatal(err)
	}
	if _, err := c.ClaimTask(ctx, agentB, idle.ID); err != nil { // agentB: idle
		t.Fatal(err)
	}
	gn := heldTask(t, c)
	if _, _, err := c.Assign(ctx, agentA, gn.ID, gone); err != nil {
		t.Fatal(err)
	}
	if _, err := c.ClaimTask(ctx, gone, gn.ID); err != nil { // takes it up, then leaves the roster
		t.Fatal(err)
	}
	queued, err := c.CreateTask(ctx, agentA, coord.NewTask{Title: "queued"})
	if err != nil {
		t.Fatal(err)
	}
	held, _ := c.CreateTask(ctx, agentA, coord.NewTask{Title: "held"})
	told, _ := c.CreateTask(ctx, agentA, coord.NewTask{Title: "told"})
	for _, tk := range []coord.Task{queued, held, told} {
		if _, _, err := c.Assign(ctx, agentA, tk.ID, agentB); err != nil {
			t.Fatal(err)
		}
	}
	open = []inbox.Delivery{
		notice(agentB, "task:"+queued.ID, inbox.Queued),
		notice(agentB, "task:"+held.ID, inbox.Held),
		notice(agentB, "task:"+told.ID, inbox.Delivered),
		notice(agentA, "task:"+told.ID, inbox.Queued),                                     // another agent's notice
		{To: agentB, State: inbox.Queued, Message: inbox.Message{Ref: "task:" + told.ID}}, // not a Board notice
		later(agentB, "task:"+told.ID, coord.NoticeUnblocked),                             // a later notice, not the hand-over
		later(agentB, "task:"+told.ID, coord.NoticeWatched),
	}
	byID := map[string]CardSummary{}
	for _, s := range b.Cards() {
		byID[s.ID] = s
	}
	cases := []struct {
		name string
		id   string
		want Holding
		who  string
	}{
		{"nobody", nobody.ID, HoldNobody, ""},
		{"busy agent", active.ID, HoldActive, agentA},
		{"idle agent", idle.ID, HoldIdle, agentB},
		{"gone agent", gn.ID, HoldGone, gone},
		{"assignment queued", queued.ID, HoldWaiting, agentB},
		{"assignment held", held.ID, HoldWaiting, agentB},
		{"assignment delivered, not taken up", told.ID, HoldAssigned, agentB},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			h := byID[tc.id].Holder
			if h.State != tc.want || h.Who.Key != tc.who {
				t.Fatalf("holder %+v, want %s %s", h, tc.want, tc.who)
			}
			if tc.want == HoldWaiting != h.Who.Waiting {
				t.Fatalf("waiting %+v", h.Who)
			}
		})
	}
	// Taking it up ends the wait, though the notice is still queued.
	if _, err := c.ClaimTask(ctx, agentB, queued.ID); err != nil {
		t.Fatal(err)
	}
	if s, _ := b.Card(queued.ID); s.Holder.State != HoldIdle || s.Holder.Who.Waiting {
		t.Fatalf("after claim %+v", s.Holder)
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
	if _, _, err := c.Answer(ctx, "user:dev-a", tk.ID, "b1", coord.BlockerAnswer{Text: "yes", Resolve: true}); err != nil {
		t.Fatal(err)
	}
	d, err := b.Card(tk.ID)
	if err != nil {
		t.Fatal(err)
	}
	if d.Short != "1" {
		t.Fatalf("short ref %q", d.Short)
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
		{"pick and wake", 1, "", true, false, "TAP", "wake", coord.Blocked},
		{"own words, next turn", -1, " both ", false, false, "both", "next_turn", coord.Blocked},
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
			// The raiser is live: the answer goes to it and the item
			// waits on it, out of the person's count.
			bl := res.Card.Blockers[0]
			if res.Card.State != tc.wantState || res.Card.OpenBlockers != 0 || res.Card.Waiting != 1 ||
				bl.State != BlockerAnswered || bl.Resolved != nil || len(bl.Thread) != 1 || bl.Thread[0].Text != tc.wantAns ||
				bl.SentTo == nil || bl.SentTo.Key != agentA {
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

// The thread merges comments, messages about the card and activity in log
// order: a message names its recipient, a comment has no "commented" line
// of its own, and past MaxThreadView the oldest entries are counted, not
// sent. A card past coord's activity cap says its history was trimmed.
func TestCardThread(t *testing.T) {
	ctx := context.Background()
	b, c, _ := cardsBoard(t)
	tk := heldTask(t, c)
	if _, _, err := c.AddComment(ctx, "user:dev-a", tk.ID, "Use the staging DB.", false); err != nil {
		t.Fatal(err)
	}
	if _, err := c.SetSteps(ctx, agentA, tk.ID, []coord.Step{{Text: "migrate"}}); err != nil {
		t.Fatal(err)
	}
	if _, err := c.RecordMessage(ctx, coord.MessageEntry{TaskID: tk.ID, MsgID: "msg_1", From: agentA, To: agentB,
		Text: "Can you review the migration?", Files: 2}); err != nil {
		t.Fatal(err)
	}
	if _, err := c.RecordMessage(ctx, coord.MessageEntry{TaskID: tk.ID, MsgID: "msg_2", From: agentB, To: agentA,
		Held: true}); err != nil {
		t.Fatal(err)
	}
	if _, err := c.RecordMessage(ctx, coord.MessageEntry{TaskID: tk.ID, MsgID: "msg_3", From: agentA, To: "dev-a/ended",
		Text: "Still there?", Gone: true, Writer: "irori"}); err != nil {
		t.Fatal(err)
	}
	if _, _, err := c.AddComment(ctx, agentB, tk.ID, "Looks good.", false); err != nil {
		t.Fatal(err)
	}
	d, err := b.Card(tk.ID)
	if err != nil {
		t.Fatal(err)
	}
	var kinds, texts []string
	for _, it := range d.Thread {
		kinds = append(kinds, it.Kind)
		texts = append(texts, it.Text)
		if it.Kind == ThreadEvent && strings.Contains(it.Text, "commented") {
			t.Fatalf("a comment's own line is in the thread: %+v", it)
		}
		if it.Device != "" {
			t.Fatalf("device chip with one device: %+v", it)
		}
	}
	// Created, claimed, the person's comment, steps, three messages, a comment.
	want := []string{ThreadEvent, ThreadEvent, ThreadComment, ThreadEvent, ThreadMessage, ThreadMessage, ThreadMessage, ThreadComment}
	if !slices.Equal(kinds, want) {
		t.Fatalf("kinds %v, want %v\ntexts %q", kinds, want, texts)
	}
	th := d.Thread
	if th[2].Who.Name != "You" || th[2].Text != "Use the staging DB." || th[2].To != nil {
		t.Fatalf("person comment %+v", th[2])
	}
	if !strings.HasPrefix(th[3].Text, "ww-release ") {
		t.Fatalf("event line %+v", th[3])
	}
	m := th[4]
	if m.ID != "msg_1" || m.Who.Name != "ww-release" || m.To == nil || m.To.Name != "ww-multiplayer" ||
		m.Text != "Can you review the migration?" || m.Files != 2 || m.Gone || m.Held {
		t.Fatalf("message %+v", m)
	}
	if h := th[5]; !h.Held || h.Text != "" || h.Who.Key != agentB || h.To.Key != agentA {
		t.Fatalf("held message %+v", h)
	}
	if g := th[6]; !g.Gone || g.Who.Key != agentA || g.To.Key != "dev-a/ended" || g.To.Status != "gone" {
		t.Fatalf("gone message %+v", g)
	}
	if d.HistoryTrimmed || d.ThreadEarlier != 0 || len(d.Comments) != 5 {
		t.Fatalf("trimmed %v earlier %d comments %d", d.HistoryTrimmed, d.ThreadEarlier, len(d.Comments))
	}
	// No null for the webview's lists.
	raw, _ := json.Marshal(d.Thread[0])
	if strings.Contains(string(raw), "null") {
		t.Fatalf("null in %s", raw)
	}

	// Enough comments to pass both caps: coord keeps MaxActivity lines,
	// and the thread sends the newest MaxThreadView.
	for i := range coord.MaxActivity {
		if _, _, err := c.AddComment(ctx, agentA, tk.ID, fmt.Sprintf("c%d", i), false); err != nil {
			t.Fatal(err)
		}
	}
	d, err = b.Card(tk.ID)
	if err != nil {
		t.Fatal(err)
	}
	total := 5 + coord.MaxActivity // every comment and message; the activity lines are all comments' now
	if !d.HistoryTrimmed || len(d.Thread) != MaxThreadView || d.ThreadEarlier != total-MaxThreadView {
		t.Fatalf("trimmed %v thread %d earlier %d", d.HistoryTrimmed, len(d.Thread), d.ThreadEarlier)
	}
	if last := d.Thread[len(d.Thread)-1]; last.Text != fmt.Sprintf("c%d", coord.MaxActivity-1) {
		t.Fatalf("newest entry %+v", last)
	}
	if len(d.Recent) != recentLines {
		t.Fatalf("recent %d", len(d.Recent))
	}
}

// A message on a card's thread opens in full for the person: the whole
// text the card cut or left off, and its files. Only that card's messages
// open.
func TestCardMessage(t *testing.T) {
	ctx := context.Background()
	b, c, _ := cardsBoard(t)
	tk := heldTask(t, c)
	other := heldTask(t, c)
	long := strings.Repeat("y", coord.MaxComment+100)
	msgs := map[string]inbox.Message{
		"msg_1": {ID: "msg_1", Text: long, Attachments: []blob.FileRef{{Name: "plan.md", Size: 12, SHA256: strings.Repeat("a", 64)}}},
		"msg_2": {ID: "msg_2", Text: "held words"},
	}
	b.message = func(id string) (inbox.Message, bool) { m, ok := msgs[id]; return m, ok }
	for _, e := range []coord.MessageEntry{
		{TaskID: tk.ID, MsgID: "msg_1", From: agentA, To: agentB, Text: long, Files: 1},
		{TaskID: tk.ID, MsgID: "msg_2", From: agentB, To: agentA, Text: "held words", Held: true},
		{TaskID: tk.ID, MsgID: "msg_9", From: agentB, To: agentA, Text: "not here"},
	} {
		if _, err := c.RecordMessage(ctx, e); err != nil {
			t.Fatal(err)
		}
	}
	cm, _, err := c.AddComment(ctx, agentA, tk.ID, "a comment", false)
	if err != nil {
		t.Fatal(err)
	}
	got, err := b.Message(tk.ID, "msg_1")
	if err != nil || got.Text != long || len(got.Files) != 1 || !strings.HasPrefix(got.Files[0], "plan.md (") {
		t.Fatalf("long: %+v %v", got, err)
	}
	if got, err := b.Message(tk.ID, " msg_2 "); err != nil || got.Text != "held words" || got.Files == nil {
		t.Fatalf("held: %+v %v", got, err)
	}
	for _, tc := range []struct{ task, id string }{
		{other.ID, "msg_1"}, // another card's message
		{tk.ID, cm.ID},      // a plain comment
		{tk.ID, "msg_404"},  // none
		{tk.ID, "msg_9"},    // on the card, not in this inbox yet
	} {
		if _, err := b.Message(tc.task, tc.id); !errors.Is(err, coord.ErrNotFound) {
			t.Errorf("%s on %s: %v, want not found", tc.id, tc.task, err)
		}
	}
}

// Entries another device wrote carry that device's chip once the person
// has a second device; this device's never do.
func TestCardThreadAcrossDevices(t *testing.T) {
	ctx := context.Background()
	st, err := store.Open(ctx, filepath.Join(t.TempDir(), "irori.db"), "dev-a", hlc.New(time.Now))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	open := func() *coord.Coord {
		t.Helper()
		c, err := coord.New(ctx, st, nil, coord.Options{})
		if err != nil {
			t.Fatal(err)
		}
		return c
	}
	c := open()
	tk := heldTask(t, c)
	const mac = "dev-b/agent-c"
	later := hlc.Timestamp(time.Now().Add(time.Hour).UnixMilli()) << 16
	row := func(seq int64, kind string, payload map[string]any) {
		t.Helper()
		raw, err := json.Marshal(payload)
		if err != nil {
			t.Fatal(err)
		}
		if err := st.Insert(ctx, store.Event{DeviceID: "dev-b", Seq: seq, HLC: later + hlc.Timestamp(seq),
			Actor: "agent:" + mac, Kind: kind, Payload: raw}); err != nil {
			t.Fatal(err)
		}
	}
	row(1, coord.KindTaskWatched, map[string]any{"taskId": tk.ID, "who": mac, "watching": true, "by": mac})
	row(2, coord.KindTaskComment, map[string]any{"taskId": tk.ID, "commentId": "msg_9", "msgId": "msg_9",
		"by": mac, "to": agentA, "text": "Port 8799 is mine for a minute.", "at": time.Now()})
	if _, _, err := c.AddComment(ctx, agentA, tk.ID, "ok", false); err != nil {
		t.Fatal(err)
	}

	b, _, _ := cardsBoard(t)
	b.coord = open() // replay folds dev-b's rows
	b.self = "dev-a"
	devs := []identity.DeviceState{
		{Cert: identity.Cert{DeviceID: "dev-a", Label: "PC"}},
		{Cert: identity.Cert{DeviceID: "dev-b", Label: "Mac"}},
	}
	b.devices = func() []identity.DeviceState { return devs }
	d, err := b.Card(tk.ID)
	if err != nil {
		t.Fatal(err)
	}
	find := func(th []CardThreadItem, id string) (int, CardThreadItem) {
		t.Helper()
		for i, it := range th {
			if it.ID == id || it.Text == id {
				return i, it
			}
		}
		t.Fatalf("no %q in %+v", id, th)
		return 0, CardThreadItem{}
	}
	_, ok := find(d.Thread, "ok")
	w, watch := find(d.Thread, "dev-b:1")
	m, msg := find(d.Thread, "msg_9")
	if m != w+1 {
		t.Fatalf("dev-b's rows out of order: watch %d, message %d", w, m)
	}
	if ok.Kind != ThreadComment || ok.Text != "ok" || ok.Device != "" {
		t.Fatalf("this device's comment %+v", ok)
	}
	if watch.Kind != ThreadEvent || watch.Device != "Mac" || watch.Who.Device != "Mac" {
		t.Fatalf("mac event %+v", watch)
	}
	if msg.Kind != ThreadMessage || msg.Device != "Mac" || msg.Who.Key != mac || msg.Who.Where != "Mac" ||
		msg.To == nil || msg.To.Name != "ww-release" || msg.To.Device != "" {
		t.Fatalf("mac message %+v", msg)
	}
	// One device again: names stay, chips go.
	devs[1].Revoked = true
	d, _ = b.Card(tk.ID)
	if _, m := find(d.Thread, "msg_9"); m.Device != "" || m.Who.Where != "Mac" {
		t.Fatalf("unpaired message %+v", m)
	}
}

// The person's answer goes to the live raiser and leaves the item open:
// answered (out of the person's count), then replied once the agent asks
// back (in it again), and only the agent's resolve with a note closes it,
// keeping the person's answer. The answer and the reply sit in the item's
// own thread; the card's thread keeps a line for each.
func TestAnswerLeavesBlockerOpen(t *testing.T) {
	ctx := context.Background()
	b, c, got := cardsBoard(t)
	tk := heldTask(t, c)
	if _, _, err := c.Raise(ctx, agentA, tk.ID, coord.NewBlocker{Question: "Which DB?",
		Options: []coord.Option{{Answer: "postgres"}}}); err != nil {
		t.Fatal(err)
	}
	res, err := b.Answer(tk.ID, "b1", -1, "Which one is cheaper to run?", true)
	if err != nil {
		t.Fatal(err)
	}
	bl := res.Card.Blockers[0]
	if res.Card.State != coord.Blocked || bl.State != BlockerAnswered || bl.Resolved != nil ||
		res.Card.OpenBlockers != 0 || res.Card.Waiting != 1 || bl.SentTo == nil || bl.SentTo.Name != "ww-release" {
		t.Fatalf("after answer %+v blocker %+v", res.Card.CardSummary, bl)
	}
	if len(res.Told.To) == 0 || res.Told.To[0] != agentA || res.Told.Mode != "wake" {
		t.Fatalf("told %+v", res.Told)
	}
	if len(*got) != 1 || !strings.Contains((*got)[0].n.Text, "It is not resolved yet") {
		t.Fatalf("routed %+v", *got)
	}
	if n := b.decideCount(); n != 0 {
		t.Fatalf("decide = %d with only an answered item", n)
	}

	// The raiser asks back: replied, back in the person's count.
	if _, _, err := c.CommentOn(ctx, agentA, tk.ID, coord.CommentInput{Text: "Postgres, but I need the budget.",
		BlockerID: "b1", Missing: []string{"monthly budget"}}); err != nil {
		t.Fatal(err)
	}
	d, err := b.Card(tk.ID)
	if err != nil {
		t.Fatal(err)
	}
	bl = d.Blockers[0]
	if bl.State != BlockerReplied || d.OpenBlockers != 1 || d.Waiting != 0 || d.BlockerQuestion != "Which DB?" ||
		!slices.Equal(bl.Missing, []string{"monthly budget"}) || bl.SentTo != nil {
		t.Fatalf("after reply %+v blocker %+v", d.CardSummary, bl)
	}
	if len(bl.Thread) != 2 || bl.Thread[0].Who.Name != "You" || bl.Thread[0].To == nil || bl.Thread[0].To.Key != agentA ||
		bl.Thread[1].Who.Key != agentA || bl.Thread[1].Text != "Postgres, but I need the budget." ||
		!slices.Equal(bl.Thread[1].Missing, []string{"monthly budget"}) || bl.Thread[1].BlockerID != "b1" {
		t.Fatalf("blocker thread %+v", bl.Thread)
	}
	// The card's thread has the lines, not the comments themselves.
	var lines []string
	for _, it := range d.Thread {
		if it.BlockerID == "b1" && it.Kind != ThreadEvent {
			t.Fatalf("blocker comment in the card's thread: %+v", it)
		}
		if it.Kind == ThreadEvent && it.BlockerID == "b1" {
			lines = append(lines, it.Text)
		}
	}
	if len(lines) != 3 || !strings.Contains(lines[1], "answered blocker b1") || !strings.Contains(lines[2], "replied on blocker b1") {
		t.Fatalf("card thread lines %q", lines)
	}
	if n := b.decideCount(); n != 1 {
		t.Fatalf("decide = %d with a replied item", n)
	}

	// Only the agent's resolve, with a note, closes it.
	if _, _, err := c.Answer(ctx, agentA, tk.ID, "b1", coord.BlockerAnswer{Text: "Going with postgres on the small plan.", Resolve: true}); err != nil {
		t.Fatal(err)
	}
	if d, err = b.Card(tk.ID); err != nil {
		t.Fatal(err)
	}
	bl = d.Blockers[0]
	if d.State != coord.Working || bl.State != BlockerResolved || bl.Resolved == nil ||
		bl.Resolved.Note != "Going with postgres on the small plan." || bl.Resolved.Answer != "Which one is cheaper to run?" ||
		d.OpenBlockers != 0 || d.Waiting != 0 || len(bl.Thread) != 2 {
		t.Fatalf("after resolve %+v blocker %+v", d.CardSummary, bl)
	}
	raw, _ := json.Marshal(bl)
	if strings.Contains(string(raw), "null") {
		t.Fatalf("null in %s", raw)
	}
}

// With nobody running to act on it, the person's answer resolves the item
// at once: the raiser has ended and no holder is live. Words added to an
// answer the raiser never acted on become the answer, not a note.
func TestAnswerResolvesWhenNobodyCanAct(t *testing.T) {
	ctx := context.Background()
	b, c, _ := cardsBoard(t)
	tk := heldTask(t, c)
	for _, q := range []string{"Ship it?", "Which region?"} {
		if _, _, err := c.Raise(ctx, agentA, tk.ID, coord.NewBlocker{Question: q}); err != nil {
			t.Fatal(err)
		}
	}
	// b2 is answered while agentA is still running.
	if _, err := b.Answer(tk.ID, "b2", -1, "Which is closest?", false); err != nil {
		t.Fatal(err)
	}
	b.agents = func() []presence.AgentCard { return nil } // agentA has ended
	res, err := b.Answer(tk.ID, "b1", -1, "yes", true)
	if err != nil {
		t.Fatal(err)
	}
	bl := blockerOf(t, res.Card, "b1")
	if bl.State != BlockerResolved || bl.Resolved.Answer != "yes" || bl.Resolved.Note != "" || res.Card.State != coord.Blocked {
		t.Fatalf("b1 %+v card %+v", bl, res.Card.CardSummary)
	}
	if res, err = b.Answer(tk.ID, "b2", -1, "eu-west", true); err != nil {
		t.Fatal(err)
	}
	bl = blockerOf(t, res.Card, "b2")
	if bl.State != BlockerResolved || bl.Resolved.Answer != "eu-west" || bl.Resolved.Note != "" ||
		res.Card.State != coord.Working || len(bl.Thread) != 2 {
		t.Fatalf("b2 %+v card %+v", bl, res.Card.CardSummary)
	}

	// A blocker the person raised has no agent to act on it either.
	tk2 := heldTask(t, c)
	if _, _, err := c.Raise(ctx, "user:dev-a", tk2.ID, coord.NewBlocker{Question: "Remind me?"}); err != nil {
		t.Fatal(err)
	}
	b.agents = func() []presence.AgentCard {
		return []presence.AgentCard{{Key: agentA, Name: "ww-release", Status: "gone"}}
	}
	if res, err = b.Answer(tk2.ID, "b1", -1, "done", false); err != nil {
		t.Fatal(err)
	}
	if bl = blockerOf(t, res.Card, "b1"); bl.State != BlockerResolved {
		t.Fatalf("person's own blocker %+v", bl)
	}
}

// A blocker the person raised has no agent to send the answer to, even
// with a live holder: the person's answer resolves it at once instead of
// leaving it waiting on nobody.
func TestAnswerResolvesPersonRaisedWithLiveHolder(t *testing.T) {
	b, c, _ := cardsBoard(t)
	tk := heldTask(t, c) // agentA holds it and is running
	if _, err := b.Block(tk.ID, "check the staging DNS", coord.BlockedOnPerson, ""); err != nil {
		t.Fatal(err)
	}
	res, err := b.Answer(tk.ID, "b1", -1, "checked", false)
	if err != nil {
		t.Fatal(err)
	}
	bl := blockerOf(t, res.Card, "b1")
	if bl.State != BlockerResolved || bl.Resolved.Answer != "checked" || res.Card.State == coord.Blocked || res.Card.Waiting != 0 {
		t.Fatalf("person's own blocker %+v card %+v", bl, res.Card.CardSummary)
	}
	if n := b.decideCount(); n != 0 {
		t.Fatalf("decide = %d", n)
	}
}

// "Mark resolved" with no note keeps the person's answer; with one, the
// note goes beside the check. Only an open item with an answer has
// nothing more to ask for.
func TestResolveBlockerByPerson(t *testing.T) {
	ctx := context.Background()
	b, c, got := cardsBoard(t)
	tk := heldTask(t, c)
	for _, q := range []string{"Which DB?", "Which port?"} {
		if _, _, err := c.Raise(ctx, agentA, tk.ID, coord.NewBlocker{Question: q,
			Options: []coord.Option{{Answer: "postgres"}, {Answer: "8080"}}}); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := b.ResolveBlocker(tk.ID, NoteItem, ""); !errors.Is(err, coord.ErrInvalid) {
		t.Fatalf("resolved the note item: %v", err)
	}
	if _, err := b.ResolveBlocker(tk.ID, "b1", ""); !errors.Is(err, coord.ErrInvalid) {
		t.Fatalf("resolved an unanswered item with nothing: %v", err)
	}
	if _, err := b.Answer(tk.ID, "b1", 0, "", false); err != nil {
		t.Fatal(err)
	}
	if _, err := b.Answer(tk.ID, "b2", -1, "whatever is free", false); err != nil {
		t.Fatal(err)
	}
	*got = nil
	res, err := b.ResolveBlocker(tk.ID, "b1", "")
	if err != nil {
		t.Fatal(err)
	}
	bl := blockerOf(t, res.Card, "b1")
	if bl.State != BlockerResolved || bl.Resolved.Answer != "postgres" || bl.Resolved.Option != 0 ||
		bl.Resolved.Note != "" || bl.Resolved.By.Name != "You" || res.Card.State != coord.Blocked {
		t.Fatalf("b1 %+v", bl)
	}
	if res.Told.Mode != "next_turn" || len(*got) != 1 || (*got)[0].n.Wake {
		t.Fatalf("told %+v routed %+v", res.Told, *got)
	}
	if res, err = b.ResolveBlocker(tk.ID, " b2 ", "I'll pick one myself later."); err != nil {
		t.Fatal(err)
	}
	bl = blockerOf(t, res.Card, "b2")
	if bl.State != BlockerResolved || bl.Resolved.Answer != "whatever is free" || bl.Resolved.Note != "I'll pick one myself later." ||
		res.Card.State != coord.Working {
		t.Fatalf("b2 %+v", bl)
	}
	if _, err := b.ResolveBlocker(tk.ID, "b2", ""); !errors.Is(err, coord.ErrBadState) {
		t.Fatalf("resolved twice: %v", err)
	}
}

// An answered item waits on an agent: it isn't counted as a decision, the
// Blocked fallback doesn't stand in for it, and the grid sorts it with the
// cards that need nothing. A replied one is a decision again.
func TestCardSummaryWaiting(t *testing.T) {
	ctx := context.Background()
	b, c, _ := cardsBoard(t)
	tk := heldTask(t, c)
	if _, _, err := c.Raise(ctx, agentA, tk.ID, coord.NewBlocker{Question: "Ship?"}); err != nil {
		t.Fatal(err)
	}
	if _, err := b.Answer(tk.ID, "b1", -1, "After lunch.", false); err != nil {
		t.Fatal(err)
	}
	g := b.Cards()
	if len(g) != 1 || g[0].State != coord.Blocked || g[0].OpenBlockers != 0 || g[0].Waiting != 1 || g[0].BlockerQuestion != "" {
		t.Fatalf("answered %+v", g)
	}
	if n := b.decideCount(); n != 0 {
		t.Fatalf("decide = %d", n)
	}
	if _, _, err := c.CommentOn(ctx, agentA, tk.ID, coord.CommentInput{Text: "Which lunch?", BlockerID: "b1"}); err != nil {
		t.Fatal(err)
	}
	if g = b.Cards(); g[0].OpenBlockers != 1 || g[0].Waiting != 0 || g[0].BlockerQuestion != "Ship?" {
		t.Fatalf("replied %+v", g)
	}
	if n := b.decideCount(); n != 1 {
		t.Fatalf("decide = %d", n)
	}
}

func blockerOf(t *testing.T, d CardDetail, id string) CardBlocker {
	t.Helper()
	for _, bl := range d.Blockers {
		if bl.ID == id {
			return bl
		}
	}
	t.Fatalf("no blocker %s in %+v", id, d.Blockers)
	return CardBlocker{}
}

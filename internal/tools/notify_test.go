package tools

import (
	"strings"
	"sync"
	"testing"
	"time"

	toast "github.com/StephenSHorton/go-toast"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/notify"
)

// cards is a fake go-toast for the notify tool.
type cards struct {
	mu   sync.Mutex
	live map[string]toast.Note
	all  []toast.Note
}

func (c *cards) Configure(toast.Config)   {}
func (c *cards) SetOptions(toast.Options) {}
func (c *cards) Post(n toast.Note) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.live[n.ID] = n
	c.all = append(c.all, n)
}
func (c *cards) Close(id string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	delete(c.live, id)
}

func (c *cards) last(t *testing.T) toast.Note {
	t.Helper()
	c.mu.Lock()
	defer c.mu.Unlock()
	if len(c.all) == 0 {
		t.Fatal("no card was posted")
	}
	return c.all[len(c.all)-1]
}

func withCards(h *harness) *cards {
	c := &cards{live: map[string]toast.Note{}}
	h.b.nf = notify.New(notify.Config{Toaster: c})
	return c
}

// answerFor waits for Irori's next_turn answer to agent and returns it.
func answerFor(t *testing.T, h *harness, agent string) inbox.Delivery {
	t.Helper()
	for i := 0; i < 300; i++ {
		for _, d := range h.b.ib.For(agent, true, 50) {
			if d.Message.From.Key == inbox.SystemKey && d.Message.Reason == "notify answer" {
				return d
			}
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("no answer reached the agent")
	return inbox.Delivery{}
}

func TestNotifyAnswerGoesBackNextTurn(t *testing.T) {
	h := newHarness(t)
	c := withCards(h)
	out := h.ok(claude(keyA, "alpha"), "notify", map[string]any{
		"text": "The migration drops the users table. Go ahead?", "title": "Drop users?", "level": "warn",
		"actions": []string{"Go ahead", "Stop"},
	})
	has(t, out, "next turn", `"Go ahead"`, `"Stop"`)
	card := c.last(t)
	if !strings.HasPrefix(card.ID, "ask:") || card.Title != "alpha: Drop users?" || card.Kind != toast.Warn || !card.Sticky {
		t.Fatalf("card = %+v", card)
	}
	if len(card.Actions) != 2 {
		t.Fatalf("actions = %+v", card.Actions)
	}
	card.Actions[1].Run() // Stop
	card.Actions[0].Run() // a second press is ignored
	card.OnDismiss()
	d := answerFor(t, h, keyA)
	if d.Mode != inbox.ModeNextTurn || !d.Message.Notice {
		t.Fatalf("answer delivery mode %q notice %v", d.Mode, d.Message.Notice)
	}
	if want := "You asked the person: Drop users?. They chose: Stop"; d.Message.Text != want {
		t.Fatalf("answer = %q; want %q", d.Message.Text, want)
	}
	n := 0
	for _, x := range h.b.ib.For(keyA, true, 50) {
		if x.Message.From.Key == inbox.SystemKey {
			n++
		}
	}
	if n != 1 {
		t.Fatalf("%d answers; want exactly one", n)
	}
}

func TestNotifyDismissedAndExpired(t *testing.T) {
	h := newHarness(t)
	c := withCards(h)
	h.ok(claude(keyA, "alpha"), "notify", map[string]any{"text": "Pick one", "actions": []string{"A", "B"}})
	c.last(t).OnDismiss()
	if got := answerFor(t, h, keyA).Message.Text; got != "You asked the person: Pick one. They dismissed it without choosing." {
		t.Fatalf("dismiss answer = %q", got)
	}

	h.set.impl.notifyState.answer = 30 * time.Millisecond
	h.ok(claude(keyB, "beta"), "notify", map[string]any{"text": "Merge now?", "title": "Merge", "actions": []string{"Yes"}})
	got := answerFor(t, h, keyB).Message.Text
	if !strings.HasPrefix(got, "You asked the person: Merge. No answer") {
		t.Fatalf("expired answer = %q", got)
	}
	c.mu.Lock()
	_, up := c.live[c.all[len(c.all)-1].ID]
	c.mu.Unlock()
	if up {
		t.Fatal("the expired card is still up")
	}
}

func TestNotifyWithoutActionsSendsNoAnswer(t *testing.T) {
	h := newHarness(t)
	c := withCards(h)
	out := h.ok(claude(keyA, "alpha"), "notify", map[string]any{"text": "Deploy finished", "level": "success"})
	has(t, out, "No answer comes back")
	card := c.last(t)
	if card.Sticky || card.Title != "alpha" || card.Body != "Deploy finished" || card.Kind != toast.Success {
		t.Fatalf("card = %+v", card)
	}
	if card.OnDismiss != nil {
		card.OnDismiss()
	}
	time.Sleep(50 * time.Millisecond)
	for _, d := range h.b.ib.For(keyA, true, 50) {
		if d.Message.From.Key == inbox.SystemKey {
			t.Fatal("an info card sent an answer")
		}
	}
}

func TestNotifyRateLimit(t *testing.T) {
	h := newHarness(t)
	withCards(h)
	a := claude(keyA, "alpha")
	for i := 0; i < NotifyLimit; i++ {
		h.ok(a, "notify", map[string]any{"text": "card"})
		h.clk.advance(time.Minute)
	}
	h.fails(a, "notify", map[string]any{"text": "one too many"}, "5 notification cards", "10 minutes")
	// Another agent has its own window.
	h.ok(claude(keyB, "beta"), "notify", map[string]any{"text": "mine"})
	// The window slides: the oldest card ages out.
	h.clk.advance(NotifyWindow - 5*time.Minute) // now 10 minutes after the first
	h.ok(a, "notify", map[string]any{"text": "again"})
	h.fails(a, "notify", map[string]any{"text": "and again"}, "limit")
}

func TestNotifyRefusals(t *testing.T) {
	h := newHarness(t)
	a := claude(keyA, "alpha")
	h.fails(a, "notify", map[string]any{"text": "hi"}, "headless") // nf nil: a headless Core
	withCards(h)
	h.fails(a, "notify", map[string]any{"text": ""}, "give text")
	h.fails(a, "notify", map[string]any{"text": "x", "level": "loud"}, "level")
	h.fails(a, "notify", map[string]any{"text": "x", "actions": []string{"a", "b", "c"}}, "at most 2")
	h.fails(a, "notify", map[string]any{"text": "x", "actions": []string{"this label is far too long for a button"}}, "one or two words")
	h.fails(a, "notify", map[string]any{"text": strings.Repeat("x", 501)}, "500")
	if _, err := h.b.nf.SetOptions(notify.Options{Enabled: false}); err != nil {
		t.Fatal(err)
	}
	h.fails(a, "notify", map[string]any{"text": "x"}, "turned off")
	h.fails(Caller{}, "notify", map[string]any{"text": "x"}, "isn't bound")
}

// A caller that has never sent or received a message and is gone by the
// time the person answers (a Grok member whose stream closed) still hears.
func TestNotifyAnswerAfterCallerLeft(t *testing.T) {
	h := newHarness(t)
	c := withCards(h)
	h.ok(claude(keyG, "grokker"), "notify", map[string]any{"text": "Ship?", "actions": []string{"Yes", "No"}})
	h.b.roster.mu.Lock()
	delete(h.b.roster.cards, keyG)
	h.b.roster.mu.Unlock()
	c.last(t).Actions[0].Run()
	if got := answerFor(t, h, keyG).Message.Text; got != "You asked the person: Ship?. They chose: Yes" {
		t.Fatalf("answer = %q", got)
	}
}

// Parallel calls (Claude Code issues tool calls at once) can't beat the
// limit: the slot is taken when the count is checked.
func TestNotifyRateLimitParallel(t *testing.T) {
	h := newHarness(t)
	c := withCards(h)
	a := claude(keyA, "alpha")
	const calls = 20
	var wg sync.WaitGroup
	var mu sync.Mutex
	okN := 0
	for i := 0; i < calls; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			out := h.call(a, "notify", map[string]any{"text": "card", "actions": []string{"Yes", "No"}})
			if !strings.HasPrefix(out, "Error: ") {
				mu.Lock()
				okN++
				mu.Unlock()
			}
		}()
	}
	wg.Wait()
	c.mu.Lock()
	posted := len(c.all)
	c.mu.Unlock()
	if okN != NotifyLimit || posted != NotifyLimit {
		t.Fatalf("%d calls accepted and %d cards posted; the limit is %d", okN, posted, NotifyLimit)
	}
}

// A card that couldn't be shown doesn't use up a slot.
func TestNotifyFailedPostFreesSlot(t *testing.T) {
	h := newHarness(t)
	withCards(h)
	a := claude(keyA, "alpha")
	for i := 0; i < NotifyLimit-1; i++ {
		h.ok(a, "notify", map[string]any{"text": "card"})
	}
	// A failed Post rolls its reservation back (unreserve); check that
	// bookkeeping directly.
	st := &h.set.impl.notifyState
	now := h.set.impl.now()
	if !st.reserve(keyA, now) {
		t.Fatal("the last slot should be free")
	}
	if st.reserve(keyA, now) {
		t.Fatal("a second reservation past the limit was granted")
	}
	st.unreserve(keyA, now)
	h.ok(a, "notify", map[string]any{"text": "fits again"})
	h.fails(a, "notify", map[string]any{"text": "over"}, "limit")
}

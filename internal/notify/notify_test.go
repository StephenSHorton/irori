package notify

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	toast "github.com/StephenSHorton/go-toast"
)

// fakeToaster records what reached go-toast and lets a test press a card.
type fakeToaster struct {
	mu     sync.Mutex
	cfg    *toast.Config
	opts   []toast.Options
	posts  []toast.Note
	closes []string
	live   map[string]toast.Note
	posted chan string // each Post's ID, for tests that wait on the watcher
}

func newFake() *fakeToaster {
	return &fakeToaster{live: map[string]toast.Note{}, posted: make(chan string, 64)}
}

func (f *fakeToaster) Configure(c toast.Config) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.cfg = &c
}

func (f *fakeToaster) SetOptions(o toast.Options) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.opts = append(f.opts, o)
	if !o.Enabled {
		clear(f.live)
	}
}

func (f *fakeToaster) Post(n toast.Note) {
	f.mu.Lock()
	f.posts = append(f.posts, n)
	f.live[n.ID] = n
	f.mu.Unlock()
	f.posted <- n.ID
}

func (f *fakeToaster) Close(id string) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.closes = append(f.closes, id)
	delete(f.live, id)
}

// card is the live card id, or fails.
func (f *fakeToaster) card(t *testing.T, id string) toast.Note {
	t.Helper()
	f.mu.Lock()
	defer f.mu.Unlock()
	n, ok := f.live[id]
	if !ok {
		t.Fatalf("no live card %q; live: %v", id, keys(f.live))
	}
	return n
}

func (f *fakeToaster) isLive(id string) bool {
	f.mu.Lock()
	defer f.mu.Unlock()
	_, ok := f.live[id]
	return ok
}

// press runs a card's button the way go-toast does: the card goes, then Run.
func (f *fakeToaster) press(t *testing.T, id string, label string) {
	t.Helper()
	n := f.card(t, id)
	f.mu.Lock()
	delete(f.live, id)
	f.mu.Unlock()
	for _, a := range n.Actions {
		if a.Label == label {
			if a.Run != nil {
				a.Run()
			}
			return
		}
	}
	t.Fatalf("card %q has no button %q", id, label)
}

func (f *fakeToaster) dismiss(t *testing.T, id string) {
	t.Helper()
	n := f.card(t, id)
	f.mu.Lock()
	delete(f.live, id)
	f.mu.Unlock()
	if n.OnDismiss != nil {
		n.OnDismiss()
	}
}

func (f *fakeToaster) postCount() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return len(f.posts)
}

func keys(m map[string]toast.Note) []string {
	var out []string
	for k := range m {
		out = append(out, k)
	}
	return out
}

func TestConfigureAndHeadless(t *testing.T) {
	t.Setenv(CaptureEnv, "")
	f := newFake()
	n := New(Config{Toaster: f})
	if f.cfg == nil {
		t.Fatal("go-toast was never configured")
	}
	if f.cfg.AppName != "Irori" || f.cfg.CornerRadius != 0 || !f.cfg.CaptureExclude || f.cfg.Palette != toast.Dark {
		t.Fatalf("config = %+v", *f.cfg)
	}
	if !n.Enabled() || n.Headless() {
		t.Fatal("desktop notifier should be enabled")
	}

	t.Setenv(CaptureEnv, "1")
	f2 := newFake()
	New(Config{Toaster: f2})
	if f2.cfg.CaptureExclude {
		t.Fatal("IRORI_TOAST_CAPTURE=1 should let captures see the cards")
	}

	// Headless never touches go-toast and never shows a card.
	f3 := newFake()
	h := New(Config{Toaster: f3, Headless: true})
	if f3.cfg != nil || len(f3.opts) != 0 {
		t.Fatal("headless configured go-toast")
	}
	if h.Post(Note{ID: "x", Title: "hi"}) || h.Enabled() || !h.Headless() {
		t.Fatal("headless posted a card")
	}
	h.Clear("x")
	if f3.postCount() != 0 || len(f3.closes) != 0 {
		t.Fatal("headless reached go-toast")
	}
	var nilN *Notifier
	if nilN.Post(Note{ID: "x", Title: "hi"}) || !nilN.Headless() {
		t.Fatal("nil notifier posted")
	}
}

func TestIDsNeverStack(t *testing.T) {
	if HeldID("dlv_1") != "held:dlv_1" || NeedsID("t1") != "needs:t1" || MessageID("dlv_2") != "msg:dlv_2" || AskID("a") != "ask:a" {
		t.Fatal("id scheme changed")
	}
	f := newFake()
	n := New(Config{Toaster: f})
	if n.Post(Note{Title: "no id"}) {
		t.Fatal("a note without an id was posted")
	}
	var first, second int
	n.Post(Note{ID: HeldID("d"), Title: "one", Actions: []Action{{Label: "A", Run: func() { first++ }}}})
	n.Post(Note{ID: HeldID("d"), Title: "two", Actions: []Action{{Label: "A", Run: func() { second++ }}}})
	if f.postCount() != 2 || len(f.live) != 1 {
		t.Fatalf("same id should replace in place: posts %d live %d", f.postCount(), len(f.live))
	}
	// The replaced card's callback is dead; only the live one's runs, once.
	f.posts[0].Actions[0].Run()
	f.press(t, HeldID("d"), "A")
	f.posts[1].Actions[0].Run()
	if first != 0 || second != 1 {
		t.Fatalf("callbacks ran first=%d second=%d; want 0, 1", first, second)
	}
	if n.Live(HeldID("d")) {
		t.Fatal("an answered card is still live")
	}
}

func TestClearRunsNoCallbacks(t *testing.T) {
	f := newFake()
	n := New(Config{Toaster: f})
	ran := 0
	n.Post(Note{ID: "c", Title: "x", OnDismiss: func() { ran++ }, OnExpire: func() { ran++ }, Expire: 20 * time.Millisecond,
		Actions: []Action{{Label: "Go", Run: func() { ran++ }}}})
	if !f.card(t, "c").Sticky {
		t.Fatal("a card with OnExpire should be sticky in go-toast; the Notifier owns its clock")
	}
	n.Clear("c")
	if f.isLive("c") || len(f.closes) != 1 {
		t.Fatal("Clear didn't close the card")
	}
	time.Sleep(60 * time.Millisecond)
	f.posts[0].Actions[0].Run()
	f.posts[0].OnDismiss()
	if ran != 0 {
		t.Fatalf("callbacks ran %d times after Clear", ran)
	}
	n.Clear("nothing") // no card: no Close
	if len(f.closes) != 1 {
		t.Fatal("Clear of an unknown id reached go-toast")
	}
}

func TestExpireOwnedByNotifier(t *testing.T) {
	f := newFake()
	n := New(Config{Toaster: f})
	got := make(chan string, 3)
	n.Post(Note{ID: "e", Title: "x", Expire: 20 * time.Millisecond,
		OnExpire:  func() { got <- "expired" },
		OnDismiss: func() { got <- "dismissed" }})
	select {
	case v := <-got:
		if v != "expired" {
			t.Fatalf("got %s", v)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("OnExpire never ran")
	}
	if f.isLive("e") {
		t.Fatal("expired card was not closed")
	}
	f.posts[0].OnDismiss() // too late: the card already expired
	select {
	case v := <-got:
		t.Fatalf("a second outcome ran: %s", v)
	case <-time.After(30 * time.Millisecond):
	}
}

func TestOptionsPersist(t *testing.T) {
	dir := t.TempDir()
	// Another setting in the file survives.
	if err := os.WriteFile(filepath.Join(dir, SettingsFile), []byte(`{"other":{"x":1}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	f := newFake()
	n := New(Config{Dir: dir, Toaster: f})
	if n.Options() != DefaultOptions() {
		t.Fatalf("fresh options = %+v", n.Options())
	}
	var heard Options
	n.OnOptions(func(o Options) { heard = o })
	got, err := n.SetOptions(Options{Enabled: false, Sound: false, Volume: 3, Corner: "middle"})
	if err != nil {
		t.Fatal(err)
	}
	want := Options{Enabled: false, Sound: false, Volume: 1, Corner: CornerTopRight}
	if got != want || heard != want {
		t.Fatalf("SetOptions = %+v, heard %+v; want %+v", got, heard, want)
	}
	if last := f.opts[len(f.opts)-1]; last.Enabled || last.Volume != 1 {
		t.Fatalf("go-toast got %+v", last)
	}
	if n.Post(Note{ID: "x", Title: "off"}) {
		t.Fatal("posted while notifications are off")
	}

	n2 := New(Config{Dir: dir, Toaster: newFake()})
	if n2.Options() != want {
		t.Fatalf("reloaded %+v; want %+v", n2.Options(), want)
	}
	raw, _ := os.ReadFile(filepath.Join(dir, SettingsFile))
	var all map[string]json.RawMessage
	if err := json.Unmarshal(raw, &all); err != nil || all["other"] == nil || all["notifications"] == nil {
		t.Fatalf("settings file = %s (%v)", raw, err)
	}

	// A partial or broken file falls back to the defaults field by field.
	os.WriteFile(filepath.Join(dir, SettingsFile), []byte(`{"notifications":{"corner":"bottom-left"}}`), 0o600)
	if o := New(Config{Dir: dir, Toaster: newFake()}).Options(); o.Corner != CornerBottomLeft || !o.Enabled || o.Volume != DefaultOptions().Volume {
		t.Fatalf("partial = %+v", o)
	}
	os.WriteFile(filepath.Join(dir, SettingsFile), []byte(`{not json`), 0o600)
	if o := New(Config{Dir: dir, Toaster: newFake()}).Options(); o != DefaultOptions() {
		t.Fatalf("broken = %+v", o)
	}
}

func TestLevels(t *testing.T) {
	for in, want := range map[string]Level{"": Info, "INFO": Info, "success": Success, "warn": Warn, "error": Error} {
		if got, ok := ParseLevel(in); !ok || got != want {
			t.Fatalf("ParseLevel(%q) = %q %v", in, got, ok)
		}
	}
	if _, ok := ParseLevel("loud"); ok {
		t.Fatal("bad level accepted")
	}
	f := newFake()
	n := New(Config{Toaster: f})
	n.Post(Note{ID: "w", Title: "x", Level: Warn})
	if f.card(t, "w").Kind != toast.Warn {
		t.Fatal("level not mapped")
	}
}

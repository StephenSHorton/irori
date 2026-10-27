package main

import (
	"errors"
	"sync"
	"testing"

	toast "github.com/StephenSHorton/go-toast"
	"github.com/StephenSHorton/irori/internal/notify"
	"github.com/StephenSHorton/irori/internal/update"
)

// cardToaster is a go-toast stand-in: it keeps the live cards and lets a
// test press a button the way go-toast does (the card closes, then Run).
type cardToaster struct {
	mu     sync.Mutex
	live   map[string]toast.Note
	posts  []toast.Note
	closes []string
}

func newCardToaster() *cardToaster { return &cardToaster{live: map[string]toast.Note{}} }

func (f *cardToaster) Configure(toast.Config)     {}
func (f *cardToaster) SetOptions(o toast.Options) {}
func (f *cardToaster) Post(n toast.Note) {
	f.mu.Lock()
	f.live[n.ID] = n
	f.posts = append(f.posts, n)
	f.mu.Unlock()
}
func (f *cardToaster) Close(id string) {
	f.mu.Lock()
	delete(f.live, id)
	f.closes = append(f.closes, id)
	f.mu.Unlock()
}
func (f *cardToaster) liveIDs() map[string]toast.Note {
	f.mu.Lock()
	defer f.mu.Unlock()
	return copyNotes(f.live)
}
func (f *cardToaster) postCount() int { f.mu.Lock(); defer f.mu.Unlock(); return len(f.posts) }

func copyNotes(m map[string]toast.Note) map[string]toast.Note {
	out := make(map[string]toast.Note, len(m))
	for k, v := range m {
		out[k] = v
	}
	return out
}

func (f *cardToaster) press(t *testing.T, id, label string) {
	t.Helper()
	f.mu.Lock()
	n, ok := f.live[id]
	delete(f.live, id)
	f.mu.Unlock()
	if !ok {
		t.Fatalf("no live card %q", id)
	}
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

func newTestCard(t *testing.T, install func() error) (*updateCard, *cardToaster, *notify.Notifier) {
	t.Helper()
	ft := newCardToaster()
	n := notify.New(notify.Config{Toaster: ft})
	return &updateCard{n: n, install: install}, ft, n
}

func ready(v string) update.Status { return update.Status{Phase: update.PhaseReady, Version: v} }

func TestUpdateCardPostsOnReady(t *testing.T) {
	c, ft, _ := newTestCard(t, nil)
	c.onStatus(update.Status{Phase: update.PhaseChecking})
	c.onStatus(update.Status{Phase: update.PhaseDownloading, Version: "0.4.0"})
	if len(ft.liveIDs()) != 0 {
		t.Fatalf("a card went up before the update was ready: %v", ft.liveIDs())
	}
	c.onStatus(ready("0.4.0"))
	n, ok := ft.liveIDs()["update:0.4.0"]
	if !ok {
		t.Fatalf("no update:0.4.0 card; live %v", ft.liveIDs())
	}
	if !n.Sticky {
		t.Error("the update card must be sticky")
	}
	if n.Title != "Irori v0.4.0 is ready" {
		t.Errorf("title %q", n.Title)
	}
	if len(n.Actions) != 2 || n.Actions[0].Label != "Install" || n.Actions[1].Label != "Later" {
		t.Errorf("buttons %+v, want Install, Later", n.Actions)
	}
	// Every later transition at ready (the tray rebuild, a second emit) must
	// not post it again.
	c.onStatus(ready("0.4.0"))
	if got := ft.postCount(); got != 1 {
		t.Errorf("posted %d times, want once per version", got)
	}
}

func TestUpdateCardInstallRestarts(t *testing.T) {
	calls := 0
	c, ft, _ := newTestCard(t, func() error { calls++; return nil })
	c.onStatus(ready("0.4.0"))
	ft.press(t, "update:0.4.0", "Install")
	if calls != 1 {
		t.Fatalf("Install ran RestartToUpdate %d times, want 1", calls)
	}
	// RestartToUpdate then reports installing; nothing comes back up.
	c.onStatus(update.Status{Phase: update.PhaseInstalling, Version: "0.4.0"})
	if len(ft.liveIDs()) != 0 {
		t.Errorf("cards still up while installing: %v", ft.liveIDs())
	}
}

func TestUpdateCardLaterJustCloses(t *testing.T) {
	calls := 0
	c, ft, _ := newTestCard(t, func() error { calls++; return nil })
	c.onStatus(ready("0.4.0"))
	ft.press(t, "update:0.4.0", "Later")
	if calls != 0 {
		t.Fatal("Later must not install now; it installs on quit")
	}
	c.onStatus(ready("0.4.0"))
	if len(ft.liveIDs()) != 0 {
		t.Errorf("Later's card came back: %v", ft.liveIDs())
	}
}

func TestUpdateCardClearedWhenInstallingElsewhere(t *testing.T) {
	c, ft, _ := newTestCard(t, nil)
	c.onStatus(ready("0.4.0"))
	// The banner's Restart now or the tray item started the install.
	c.onStatus(update.Status{Phase: update.PhaseInstalling, Version: "0.4.0"})
	if _, up := ft.liveIDs()["update:0.4.0"]; up {
		t.Fatal("the card stayed up after the install started")
	}
}

func TestUpdateCardSuperseded(t *testing.T) {
	c, ft, _ := newTestCard(t, nil)
	c.onStatus(ready("0.4.0"))
	c.onStatus(ready("0.5.0"))
	live := ft.liveIDs()
	if _, up := live["update:0.4.0"]; up {
		t.Error("the 0.4.0 card stayed up after 0.5.0 replaced it")
	}
	if _, up := live["update:0.5.0"]; !up {
		t.Errorf("no 0.5.0 card; live %v", live)
	}
}

func TestUpdateCardInstallFailureRepostsWithReason(t *testing.T) {
	c, ft, _ := newTestCard(t, func() error { return errors.New("access denied") })
	c.onStatus(ready("0.4.0"))
	ft.press(t, "update:0.4.0", "Install")
	// RestartToUpdate reverts to ready on a failed launch.
	c.onStatus(ready("0.4.0"))
	n, ok := ft.liveIDs()["update:0.4.0"]
	if !ok {
		t.Fatal("a failed Install left no card to retry from")
	}
	if !n.Silent || n.Kind != toast.Warn {
		t.Errorf("retry card silent=%v kind=%v, want silent warn", n.Silent, n.Kind)
	}
	if got := ft.postCount(); got != 2 {
		t.Errorf("posted %d cards, want the first and the one retry", got)
	}
}

func TestUpdateCardRespectsNotificationsOff(t *testing.T) {
	c, ft, n := newTestCard(t, nil)
	o := n.Options()
	o.Enabled = false
	if _, err := n.SetOptions(o); err != nil {
		t.Fatal(err)
	}
	c.onStatus(ready("0.4.0"))
	if ft.postCount() != 0 {
		t.Fatal("a card went up with notifications off")
	}
	// Turned back on, the next ready transition still announces it.
	o.Enabled = true
	if _, err := n.SetOptions(o); err != nil {
		t.Fatal(err)
	}
	c.onStatus(ready("0.4.0"))
	if _, up := ft.liveIDs()["update:0.4.0"]; !up {
		t.Error("no card after notifications came back on")
	}
}

func TestUpdateCardHeadlessNeverPosts(t *testing.T) {
	ft := newCardToaster()
	n := notify.New(notify.Config{Headless: true, Toaster: ft})
	c := &updateCard{n: n}
	c.onStatus(ready("0.4.0"))
	if ft.postCount() != 0 {
		t.Fatal("a headless Notifier showed the update card")
	}
}

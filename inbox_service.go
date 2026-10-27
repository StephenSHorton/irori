package main

import (
	"context"
	"errors"
	"strings"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// EventInboxChanged carries an InboxCounts whenever a delivery or the policy
// changes. Bursts are coalesced (at most inboxEmitEvery apart), so the UI
// re-reads List rather than applying each change.
const EventInboxChanged = "inbox:changed"

// inboxEmitEvery bounds inbox:changed to 4 events a second.
const inboxEmitEvery = 250 * time.Millisecond

func init() {
	application.RegisterEvent[InboxCounts](EventInboxChanged)
}

// InboxCounts is the inbox:changed payload and the tray's numbers.
//
// Unread counts deliveries an agent hasn't picked up yet: queued, or handed
// to a wake but not yet engaged (delivered). Held counts deliveries policy
// stopped that wait for the person.
type InboxCounts struct {
	Unread int  `json:"unread"`
	Held   int  `json:"held"`
	Paused bool `json:"paused"`
}

// InboxService is the person's view of the inbox. Every action acts as the
// person at this device (core.UserSender), never as an agent.
// JS binding name: InboxService.*
type InboxService struct {
	core *core.Core

	mu      sync.Mutex
	onCount []func(InboxCounts) // tray and other in-process listeners
	kick    chan struct{}       // nudges the pump after a GUI-side policy change
}

func newInboxService(c *core.Core) *InboxService {
	return &InboxService{core: c, kick: make(chan struct{}, 1)}
}

// List returns recent deliveries across every recipient, newest first; at
// most limit (default inbox.DefaultInboxList, max 500).
func (s *InboxService) List(limit int) []inbox.Delivery {
	if limit <= 0 {
		limit = inbox.DefaultInboxList
	}
	limit = min(limit, 500)
	return nonNil(s.core.Inbox().List(limit))
}

// Held returns every delivery waiting on the person, newest first.
func (s *InboxService) Held() []inbox.Delivery {
	out := []inbox.Delivery{}
	for _, d := range s.core.Inbox().List(0) {
		if d.State == inbox.Held {
			out = append(out, d)
		}
	}
	return out
}

// Counts returns the unread and held numbers and whether wakes are paused.
func (s *InboxService) Counts() InboxCounts { return s.counts() }

// Send sends text from the person. With repo set it goes to that repo's
// front desk; otherwise to is an agent key, key prefix or name.
func (s *InboxService) Send(to, repo, text string) (inbox.Message, error) {
	to, repo = strings.TrimSpace(to), strings.TrimSpace(repo)
	addr := inbox.Address{Kind: inbox.ToAgent, Value: to}
	if repo != "" {
		addr = inbox.Address{Kind: inbox.ToRepo, Value: repo}
	} else if to == "" {
		return inbox.Message{}, errors.New("pick an agent or a repo to send to")
	}
	m, _, err := s.core.Inbox().Send(context.Background(), s.core.UserSender(), addr, text, "")
	return m, err
}

// Release delivers a held delivery past the policy (the person's Deliver).
func (s *InboxService) Release(deliveryID string) error {
	return s.core.Inbox().Release(context.Background(), deliveryID, s.by())
}

// Dismiss closes a held delivery without delivering it.
func (s *InboxService) Dismiss(deliveryID string) error {
	return s.core.Inbox().Dismiss(context.Background(), deliveryID, s.by())
}

// AllowAndRelease adds the held delivery's sender to the allow list, then
// delivers it: "Allow sender & deliver". The sender is added by key, so a
// name an agent chose for itself never widens the allow list.
func (s *InboxService) AllowAndRelease(deliveryID string) error {
	d, ok := s.core.Inbox().Get(deliveryID)
	if !ok {
		return inbox.ErrNotFound
	}
	if d.State != inbox.Held {
		return inbox.ErrBadState
	}
	key := d.Message.From.Key
	if key == "" {
		return errors.New("that delivery has no sender key")
	}
	p := s.core.Inbox().Policy()
	if !containsFold(p.Allow, key) {
		p.Allow = append(p.Allow, key)
		if err := s.SetPolicy(p); err != nil {
			return err
		}
	}
	return s.Release(deliveryID)
}

// Policy returns the wake policy.
func (s *InboxService) Policy() inbox.Policy { return s.core.Inbox().Policy() }

// SetPolicy replaces the wake policy as the person.
func (s *InboxService) SetPolicy(p inbox.Policy) error {
	if err := s.core.Inbox().SetPolicy(context.Background(), p, s.by()); err != nil {
		return err
	}
	s.nudge()
	return nil
}

// SetPaused turns pause-all on or off, keeping the rest of the policy.
func (s *InboxService) SetPaused(paused bool) error {
	p := s.core.Inbox().Policy()
	if p.Paused == paused {
		return nil
	}
	p.Paused = paused
	return s.SetPolicy(p)
}

// Paused reports whether every wake is paused.
func (s *InboxService) Paused() bool { return s.core.Inbox().Policy().Paused }

func (s *InboxService) by() string { return s.core.UserSender().Key }

// onCounts registers fn for every count change (the tray). Not bound to JS
// because it is unexported.
func (s *InboxService) onCounts(fn func(InboxCounts)) {
	s.mu.Lock()
	s.onCount = append(s.onCount, fn)
	s.mu.Unlock()
	fn(s.counts())
}

func (s *InboxService) counts() InboxCounts {
	c := InboxCounts{Paused: s.core.Inbox().Policy().Paused}
	for _, d := range s.core.Inbox().List(0) {
		switch d.State {
		case inbox.Queued, inbox.Delivered:
			c.Unread++
		case inbox.Held:
			c.Held++
		}
	}
	return c
}

// nudge asks the pump to emit, for changes that may not reach Subscribe
// (a policy write that re-queues nothing).
func (s *InboxService) nudge() {
	select {
	case s.kick <- struct{}{}:
	default:
	}
}

// pump turns inbox changes into inbox:changed, coalesced so a burst becomes
// one event at most inboxEmitEvery after the previous one.
func (s *InboxService) pump(ctx context.Context, app *application.App) {
	changes := s.core.Inbox().Subscribe(ctx)
	var last time.Time
	timer := time.NewTimer(time.Hour)
	timer.Stop()
	pending := false
	emit := func() {
		pending = false
		last = time.Now()
		c := s.counts()
		app.Event.Emit(EventInboxChanged, c)
		s.mu.Lock()
		fns := append([]func(InboxCounts){}, s.onCount...)
		s.mu.Unlock()
		for _, fn := range fns {
			fn(c)
		}
	}
	schedule := func() {
		if pending {
			return
		}
		pending = true
		timer.Reset(max(0, inboxEmitEvery-time.Since(last)))
	}
	for {
		select {
		case <-ctx.Done():
			timer.Stop()
			return
		case _, ok := <-changes:
			if !ok {
				return
			}
			schedule()
		case <-s.kick:
			schedule()
		case <-timer.C:
			emit()
		}
	}
}

func containsFold(list []string, s string) bool {
	for _, e := range list {
		if strings.EqualFold(strings.TrimSpace(e), s) {
			return true
		}
	}
	return false
}

// nonNil keeps JSON arrays from arriving in the UI as null.
func nonNil[T any](s []T) []T {
	if s == nil {
		return []T{}
	}
	return s
}

package main

import (
	"context"
	"errors"
	"fmt"
	"log"
	"strings"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/tools"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// EventInboxChanged carries an InboxCounts whenever a delivery or the policy
// changes. Bursts are coalesced (at most inboxEmitEvery apart), so the UI
// re-reads List rather than applying each change.
const EventInboxChanged = "inbox:changed"

// inboxEmitEvery bounds inbox:changed to 4 events a second.
const inboxEmitEvery = 250 * time.Millisecond

// scanAll is the List limit for a pass over every delivery; List(0) means
// the default page of 50, not all.
const scanAll = 1 << 30

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
	src  inboxSource // c.Inbox(); a seam for tests

	mu      sync.Mutex
	onCount []func(InboxCounts) // tray and other in-process listeners
	kick    chan struct{}       // nudges the pump after a GUI-side policy change
	snap    *inboxSnapshot      // the pump's last scan; nil before the first
}

// inboxSource is the part of *inbox.Inbox a scan reads.
type inboxSource interface {
	List(limit int) []inbox.Delivery
	Policy() inbox.Policy
}

// inboxSnapshot is one pass over every delivery: the counts and the held
// deliveries. The inbox has no counts or held-only query, so a pass copies
// every delivery; the pump makes one per coalesced change and Counts and
// Held serve it, instead of each making its own.
type inboxSnapshot struct {
	counts InboxCounts
	held   []inbox.Delivery
}

func newInboxService(c *core.Core) *InboxService {
	return &InboxService{core: c, src: c.Inbox(), kick: make(chan struct{}, 1)}
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

// Held returns every delivery waiting on the person, newest first, as of
// the last inbox:changed.
func (s *InboxService) Held() []inbox.Delivery {
	return append([]inbox.Delivery{}, s.snapshot().held...)
}

// HeldHistory returns deliveries a hold once kept that are no longer held
// (released, dismissed, let through by an unpause), the most recently
// decided first; at most limit (default inbox.DefaultInboxList, max 500).
// A dismissed one can still be released (Release).
func (s *InboxService) HeldHistory(limit int) []inbox.Delivery {
	if limit <= 0 {
		limit = inbox.DefaultInboxList
	}
	return nonNil(s.core.Inbox().HeldHistory(min(limit, 500)))
}

// Counts returns the unread and held numbers and whether wakes are paused,
// as of the last inbox:changed.
func (s *InboxService) Counts() InboxCounts { return s.snapshot().counts }

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

// Get returns one delivery, for the window's message dialog: a
// notification's Open names it (a held message, or one sent to the person).
func (s *InboxService) Get(deliveryID string) (inbox.Delivery, error) {
	d, ok := s.core.Inbox().Get(strings.TrimSpace(deliveryID))
	if !ok {
		return inbox.Delivery{}, fmt.Errorf("%w: delivery %q", inbox.ErrNotFound, deliveryID)
	}
	return d, nil
}

// Reply answers a message sent to the person; the sender is woken with it.
// A reply to a message about a card also goes on that card's thread; that
// failing is logged, never the reply's error.
func (s *InboxService) Reply(deliveryID, text string) error {
	ctx := context.Background()
	m, nd, err := s.core.Inbox().Reply(ctx, s.core.UserSender(), strings.TrimSpace(deliveryID), text)
	if err == nil && m.Ref != "" {
		_, rerr := tools.RecordOnCard(ctx, s.core.Coord(), m, nd.To, tools.Record{Held: tools.HeldOnCard(s.core.Inbox(), nd)})
		if rerr != nil && !errors.Is(rerr, tools.ErrNoCard) && !errors.Is(rerr, coord.ErrNotFound) {
			log.Printf("irori: put the reply on %s: %v", m.Ref, rerr)
		}
	}
	return s.acted(err)
}

// MarkRead closes a message sent to the person without replying.
func (s *InboxService) MarkRead(deliveryID string) error {
	return s.acted(s.core.Inbox().Skip(context.Background(), s.by(), strings.TrimSpace(deliveryID), "read in the Irori window"))
}

// Release delivers a held delivery past the policy (the person's Deliver),
// or a dismissed one after all (the Held list's undo).
func (s *InboxService) Release(deliveryID string) error {
	return s.acted(s.core.Inbox().Release(context.Background(), deliveryID, s.by()))
}

// Dismiss closes a held delivery without delivering it.
func (s *InboxService) Dismiss(deliveryID string) error {
	return s.acted(s.core.Inbox().Dismiss(context.Background(), deliveryID, s.by()))
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
	// Allow only lifts these two holds. A block outranks the allow list, and
	// a pause or hop limit isn't about the sender: use Release for those.
	if d.Hold != inbox.HoldNotAllowed && d.Hold != inbox.HoldBypassReceiver {
		return fmt.Errorf("%w: allowing the sender doesn't lift a %q hold", inbox.ErrBadState, d.Hold)
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
	if err := s.acted(s.core.Inbox().SetPolicy(context.Background(), p, s.by())); err != nil {
		return err
	}
	s.nudge()
	return nil
}

// acted drops the snapshot after the person's own change succeeded, so a
// read right after it is live rather than up to inboxEmitEvery old.
func (s *InboxService) acted(err error) error {
	if err == nil {
		s.mu.Lock()
		s.snap = nil
		s.mu.Unlock()
	}
	return err
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
	fn(s.snapshot().counts)
}

// snapshot returns the last scan, scanning first if there is none yet.
func (s *InboxService) snapshot() *inboxSnapshot {
	s.mu.Lock()
	snap := s.snap
	s.mu.Unlock()
	if snap != nil {
		return snap
	}
	return s.refresh()
}

// refresh scans every delivery once and keeps the result for Counts and
// Held until the next change.
func (s *InboxService) refresh() *inboxSnapshot {
	snap := &inboxSnapshot{counts: InboxCounts{Paused: s.src.Policy().Paused}, held: []inbox.Delivery{}}
	for _, d := range s.src.List(scanAll) {
		switch d.State {
		case inbox.Queued, inbox.Delivered:
			snap.counts.Unread++
		case inbox.Held:
			snap.counts.Held++
			snap.held = append(snap.held, d)
		}
	}
	s.mu.Lock()
	s.snap = snap
	s.mu.Unlock()
	return snap
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
		c := s.refresh().counts
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

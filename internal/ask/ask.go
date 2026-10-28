// Package ask is the person's approvals as rows (design/v0.4-devices.md
// §6.7, §6.3): an agent's question with buttons (the notify tool), and the
// host's question before it revives a session for another device. Every
// desktop device shows the card; any of them may answer; only the asking
// device decides:
//
//	ask.posted   {askId, by, name, title, text, level, actions, until, target?, deliveryId?}
//	             from the asking device
//	ask.answered {askId, choice, target?, deliveryId?, by, atDevice}
//	             from any GUI, actor "user"
//	ask.resolved {askId, choice, answered, answerRef}
//	             from the asking device: the first answer it folded, or none
//	             by until; every device shows it and clears the card
//
// A revive from another device starts as revive.requested {target,
// deliveryId, by, atDevice} (actor "user"); the target's host posts the
// ask, and wakes nothing until an answer names that target and delivery.
package ask

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"slices"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/owner"
	"github.com/StephenSHorton/irori/internal/store"
)

// Log kinds.
const (
	KindPosted          = "ask.posted"
	KindAnswered        = "ask.answered"
	KindResolved        = "ask.resolved"
	KindReviveRequested = "revive.requested"
)

var kinds = []string{KindPosted, KindAnswered, KindResolved, KindReviveRequested}

// The revive ask's buttons; the first one revives.
const (
	ChoiceRevive = "Revive"
	ChoiceNotNow = "Not now"
)

// ReviveWait is how long a revive ask waits for an answer.
const ReviveWait = 30 * time.Minute

// Ask is one question to the person.
type Ask struct {
	ID      string    `json:"askId"`
	By      string    `json:"by"`   // the asking agent's key, or "irori" for a revive
	Name    string    `json:"name"` // who asks, for the card
	Title   string    `json:"title,omitempty"`
	Text    string    `json:"text"`
	Level   string    `json:"level,omitempty"`
	Actions []string  `json:"actions"`
	Until   time.Time `json:"until"`
	// Target and DeliveryID are set on a revive ask: the session to revive
	// and the message it is for.
	Target     string `json:"target,omitempty"`
	DeliveryID string `json:"deliveryId,omitempty"`

	// From the fold, not the posted row.
	Device    string `json:"device"` // the asking device: it decides
	Resolved  bool   `json:"resolved"`
	Choice    string `json:"choice,omitempty"`   // the answer; "" when dismissed or unanswered
	Answered  bool   `json:"answered,omitempty"` // someone answered (else it ran out)
	AnswerRef string `json:"answerRef,omitempty"`
}

type answeredRow struct {
	AskID      string    `json:"askId"`
	Choice     string    `json:"choice"`
	Target     string    `json:"target,omitempty"`
	DeliveryID string    `json:"deliveryId,omitempty"`
	By         string    `json:"by"`
	AtDevice   string    `json:"atDevice,omitempty"`
	At         time.Time `json:"at,omitzero"`
}

type resolvedRow struct {
	AskID     string    `json:"askId"`
	Choice    string    `json:"choice"`
	Answered  bool      `json:"answered"`
	AnswerRef string    `json:"answerRef,omitempty"`
	At        time.Time `json:"at,omitzero"`
}

type reviveRow struct {
	Target     string    `json:"target"`
	DeliveryID string    `json:"deliveryId,omitempty"`
	By         string    `json:"by"`
	AtDevice   string    `json:"atDevice,omitempty"`
	At         time.Time `json:"at,omitzero"`
}

// answer is one ask.answered row as folded.
type answer struct {
	answeredRow
	ref string // "<device>/<seq>"
}

// Options configure a Book.
type Options struct {
	Store *store.Store
	Self  string
	// IsOwner reports the person's keys (identity.Chain.IsOwner): only
	// their answers and requests count.
	IsOwner func(key string) bool
	Now     func() time.Time
	// Revive wakes a session this device hosts once a revive ask was
	// answered Revive for it (the GUI's Revive). Nil: nothing to revive
	// with (headless), so a revive ask resolves without waking.
	Revive func(ctx context.Context, target string) error
	// Name shows an agent key as its display name; nil shows the key.
	Name func(key string) string
}

// Book is every ask the log holds. Safe for concurrent use.
type Book struct {
	o Options

	mu         sync.Mutex
	asks       map[string]*Ask
	answers    map[string][]answer // ask id -> answers, fold order
	last       map[string]int64
	ignored    int
	seq        uint64
	onResolved []func(Ask)
	subs       map[chan string]struct{}
	replaying  bool
	pending    []string // ask ids changed, for subscribers
	revives    bool     // a revive.requested was folded since the last duties
}

// Start replays the asks and keeps folding until ctx ends; as the asking
// device it resolves its own asks on their first answer or at until.
func Start(ctx context.Context, o Options) (*Book, error) {
	if o.Now == nil {
		o.Now = time.Now
	}
	if o.IsOwner == nil {
		self := "user:" + o.Self
		o.IsOwner = func(k string) bool { return k == self }
	}
	b := &Book{o: o, asks: map[string]*Ask{}, answers: map[string][]answer{}, last: map[string]int64{}, subs: map[chan string]struct{}{}, revives: true}
	sub := o.Store.Subscribe(ctx)
	if err := b.replay(ctx); err != nil {
		return nil, err
	}
	go b.run(ctx, sub)
	return b, nil
}

func (b *Book) replay(ctx context.Context) error {
	b.mu.Lock()
	b.replaying = true
	f := store.Filter{Kinds: kinds, Limit: store.MaxListLimit}
	for {
		evs, err := b.o.Store.List(ctx, f)
		if err != nil {
			b.replaying = false
			b.mu.Unlock()
			return fmt.Errorf("ask: replay: %w", err)
		}
		for _, ev := range evs {
			b.fold(ev)
		}
		if len(evs) < f.Limit {
			break
		}
		f.After = evs[len(evs)-1].Cursor()
	}
	b.replaying = false
	b.pending = nil
	b.mu.Unlock()
	return nil
}

func (b *Book) run(ctx context.Context, sub <-chan store.Event) {
	b.duties(ctx) // what waited while this device was off
	t := time.NewTicker(time.Second)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case ev, ok := <-sub:
			if !ok {
				return
			}
			if ev.Seq == 0 {
				b.catchUp(ctx)
				continue
			}
			if ev.DeviceID == b.o.Self || !isKind(ev.Kind) {
				continue
			}
			b.mu.Lock()
			b.fold(ev)
			b.mu.Unlock()
			b.duties(ctx)
			b.flush()
		case <-t.C:
			b.duties(ctx)
			b.flush()
		}
	}
}

func isKind(k string) bool {
	for _, x := range kinds {
		if x == k {
			return true
		}
	}
	return false
}

func (b *Book) catchUp(ctx context.Context) {
	vec, err := b.o.Store.Vector(ctx)
	if err != nil {
		return
	}
	for dev, top := range vec {
		if dev == b.o.Self {
			continue
		}
		for {
			b.mu.Lock()
			after := b.last[dev]
			b.mu.Unlock()
			if after >= top {
				break
			}
			evs, err := b.o.Store.List(ctx, store.Filter{DeviceID: dev, AfterSeq: after, Kinds: kinds, Limit: store.MaxListLimit})
			if err != nil || len(evs) == 0 {
				break
			}
			b.mu.Lock()
			for _, ev := range evs {
				b.fold(ev)
			}
			b.mu.Unlock()
			if len(evs) < store.MaxListLimit {
				break
			}
		}
	}
	b.duties(ctx)
	b.flush()
}

// fold applies one row. b.mu is held.
func (b *Book) fold(ev store.Event) {
	if ev.Seq <= b.last[ev.DeviceID] {
		return
	}
	b.last[ev.DeviceID] = ev.Seq
	switch ev.Kind {
	case KindPosted:
		var a Ask
		if json.Unmarshal(ev.Payload, &a) != nil || a.ID == "" {
			return
		}
		if a.By != "irori" && owner.DeviceOf(a.By) != ev.DeviceID {
			b.ignored++ // an agent asks only from its own device
			return
		}
		if _, ok := b.asks[a.ID]; ok {
			return
		}
		a.Device, a.Resolved, a.Choice, a.Answered, a.AnswerRef = ev.DeviceID, false, "", false, ""
		b.asks[a.ID] = &a
		b.changed(a.ID)
	case KindAnswered:
		var r answeredRow
		if json.Unmarshal(ev.Payload, &r) != nil || r.AskID == "" {
			return
		}
		// Only the person answers, from a GUI (§2).
		if ev.Actor != "user" || !b.o.IsOwner(r.By) {
			b.ignored++
			return
		}
		b.answers[r.AskID] = append(b.answers[r.AskID], answer{answeredRow: r, ref: ev.DeviceID + "/" + strconv.FormatInt(ev.Seq, 10)})
		b.changed(r.AskID)
	case KindResolved:
		var r resolvedRow
		if json.Unmarshal(ev.Payload, &r) != nil {
			return
		}
		a := b.asks[r.AskID]
		if a == nil || a.Device != ev.DeviceID {
			b.ignored++ // only the asking device decides (§6.0)
			return
		}
		if a.Resolved {
			return
		}
		a.Resolved, a.Choice, a.Answered, a.AnswerRef = true, r.Choice, r.Answered, r.AnswerRef
		b.changed(a.ID)
	case KindReviveRequested:
		b.revives = true // duties posts the host's ask
	}
}

func (b *Book) changed(id string) {
	if !b.replaying {
		b.pending = append(b.pending, id)
	}
}

func (b *Book) flush() {
	b.mu.Lock()
	ids := b.pending
	b.pending = nil
	for _, id := range ids {
		for ch := range b.subs {
			select {
			case ch <- id:
			default:
			}
		}
	}
	b.mu.Unlock()
}

// duties is the asking device's part: resolve each own ask on the first
// answer folded or at until, post the ask for each revive requested of a
// session this device hosts, and run what resolved.
func (b *Book) duties(ctx context.Context) {
	b.mu.Lock()
	revives := b.revives
	b.revives = false
	b.mu.Unlock()
	if revives {
		if err := b.postRevives(ctx); err != nil && ctx.Err() == nil {
			log.Printf("ask: revive requests: %v", err)
			b.mu.Lock()
			b.revives = true
			b.mu.Unlock()
		}
	}
	now := b.o.Now()
	b.mu.Lock()
	var todo []*Ask
	for _, a := range b.asks {
		if a.Device == b.o.Self && !a.Resolved {
			todo = append(todo, a)
		}
	}
	sort.Slice(todo, func(i, j int) bool { return todo[i].ID < todo[j].ID })
	var done []Ask
	for _, a := range todo {
		r := resolvedRow{AskID: a.ID, At: now}
		if ans := b.answers[a.ID]; len(ans) > 0 {
			first := ans[0]
			r.Choice, r.Answered, r.AnswerRef = first.Choice, true, first.ref
		} else if now.Before(a.Until) {
			continue
		}
		if _, err := b.o.Store.Append(ctx, "irori", KindResolved, r); err != nil {
			if ctx.Err() == nil {
				log.Printf("ask: resolve %s: %v", a.ID, err)
			}
			continue
		}
		a.Resolved, a.Choice, a.Answered, a.AnswerRef = true, r.Choice, r.Answered, r.AnswerRef
		b.pending = append(b.pending, a.ID)
		done = append(done, *a)
	}
	fns := slices.Clone(b.onResolved)
	b.mu.Unlock()
	for _, a := range done {
		if a.Target != "" {
			b.reviveIfAnswered(ctx, a)
			continue
		}
		for _, fn := range fns {
			fn(a)
		}
	}
}

// reviveIfAnswered wakes a revive ask's target only on an answer that
// names exactly its target and delivery (§6.3).
func (b *Book) reviveIfAnswered(ctx context.Context, a Ask) {
	if !a.Answered || a.Choice != ChoiceRevive || b.o.Revive == nil {
		return
	}
	b.mu.Lock()
	var first *answer
	if ans := b.answers[a.ID]; len(ans) > 0 {
		first = &ans[0]
	}
	b.mu.Unlock()
	if first == nil || first.Target != a.Target || first.DeliveryID != a.DeliveryID {
		log.Printf("ask: a revive answer for %s named another target; nothing woken", a.ID)
		return
	}
	if err := b.o.Revive(ctx, a.Target); err != nil {
		log.Printf("ask: revive %s: %v", a.Target, err)
	}
}

// postRevives turns each revive.requested for a session this device hosts
// into an ask, once.
func (b *Book) postRevives(ctx context.Context) error {
	evs, err := b.o.Store.List(ctx, store.Filter{Kinds: []string{KindReviveRequested}, Limit: store.MaxListLimit, Desc: true})
	if err != nil {
		return err
	}
	for _, ev := range evs {
		var r reviveRow
		if json.Unmarshal(ev.Payload, &r) != nil || r.Target == "" || owner.DeviceOf(r.Target) != b.o.Self {
			continue
		}
		if ev.Actor != "user" || !b.o.IsOwner(r.By) {
			continue
		}
		id := "revive-" + ev.DeviceID + "-" + strconv.FormatInt(ev.Seq, 10)
		b.mu.Lock()
		_, posted := b.asks[id]
		b.mu.Unlock()
		if posted {
			continue
		}
		name := r.Target
		if b.o.Name != nil {
			if n := strings.TrimSpace(b.o.Name(r.Target)); n != "" {
				name = n
			}
		}
		a := Ask{ID: id, By: "irori", Name: "Irori", Title: "Revive " + name + "?",
			Text:    "Your other device asked to revive " + name + " here, to deliver a message to it.",
			Actions: []string{ChoiceRevive, ChoiceNotNow}, Until: b.o.Now().Add(ReviveWait),
			Target: r.Target, DeliveryID: r.DeliveryID}
		if _, err := b.post(ctx, a); err != nil {
			return err
		}
	}
	return nil
}

// Post records an agent's question (the notify tool). The card shows on
// every desktop device; the first answer folded here resolves it, and
// OnResolved runs then (or at until with no answer).
func (b *Book) Post(ctx context.Context, a Ask) (Ask, error) {
	if a.By == "" || a.Text == "" || len(a.Actions) == 0 || a.Until.IsZero() {
		return Ask{}, errors.New("ask: an ask needs who asks, text, actions and an end")
	}
	return b.post(ctx, a)
}

func (b *Book) post(ctx context.Context, a Ask) (Ask, error) {
	if a.ID == "" {
		b.mu.Lock()
		b.seq++
		a.ID = strconv.FormatInt(b.o.Now().UnixNano(), 36) + "-" + strconv.FormatUint(b.seq, 36)
		b.mu.Unlock()
	}
	a.Device = ""
	ev, err := b.o.Store.Append(ctx, "irori", KindPosted, a)
	if err != nil {
		return Ask{}, err
	}
	b.mu.Lock()
	b.fold(ev)
	out := *b.asks[a.ID]
	b.mu.Unlock()
	b.flush()
	return out, nil
}

// ErrNotPerson: only the person answers an ask or asks for a revive.
var ErrNotPerson = errors.New("ask: only the person answers")

// Answer is the person's choice on any device ("" = dismissed). For a
// revive ask it names the ask's target and delivery, as the host checks.
func (b *Book) Answer(ctx context.Context, id, choice, by string) error {
	if !b.o.IsOwner(by) {
		return ErrNotPerson
	}
	b.mu.Lock()
	a := b.asks[id]
	var target, delivery string
	if a != nil {
		target, delivery = a.Target, a.DeliveryID
	}
	b.mu.Unlock()
	if a == nil {
		return fmt.Errorf("ask: no ask %q", id)
	}
	ev, err := b.o.Store.Append(ctx, "user", KindAnswered, answeredRow{
		AskID: id, Choice: choice, Target: target, DeliveryID: delivery, By: by, AtDevice: b.o.Self, At: b.o.Now(),
	})
	if err != nil {
		return err
	}
	b.mu.Lock()
	b.fold(ev)
	b.mu.Unlock()
	b.duties(ctx)
	b.flush()
	return nil
}

// RequestRevive asks target's host to revive it for deliveryID (§6.3).
// by is the person; a session on this device is revived here directly.
func (b *Book) RequestRevive(ctx context.Context, target, deliveryID, by string) error {
	if !b.o.IsOwner(by) {
		return ErrNotPerson
	}
	if dev := owner.DeviceOf(target); dev == "" || dev == b.o.Self {
		return errors.New("ask: that session is on this device; revive it here")
	}
	_, err := b.o.Store.Append(ctx, "user", KindReviveRequested, reviveRow{
		Target: target, DeliveryID: deliveryID, By: by, AtDevice: b.o.Self, At: b.o.Now(),
	})
	return err
}

// OnResolved runs fn for each of this device's agent asks once it resolves
// (on this device only: the asking device delivers the answer).
func (b *Book) OnResolved(fn func(Ask)) {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.onResolved = append(b.onResolved, fn)
}

// Get returns one ask.
func (b *Book) Get(id string) (Ask, bool) {
	b.mu.Lock()
	defer b.mu.Unlock()
	a := b.asks[id]
	if a == nil {
		return Ask{}, false
	}
	return *a, true
}

// Open lists the asks not resolved yet, oldest first.
func (b *Book) Open() []Ask {
	b.mu.Lock()
	defer b.mu.Unlock()
	out := []Ask{}
	for _, a := range b.asks {
		if !a.Resolved {
			out = append(out, *a)
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

// Ignored counts rows refused by the writer rules.
func (b *Book) Ignored() int {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.ignored
}

// Subscribe sends the id of each ask that changed (posted, answered,
// resolved) until ctx ends. A slow reader drops ids and should re-read
// Open.
func (b *Book) Subscribe(ctx context.Context) <-chan string {
	ch := make(chan string, 64)
	b.mu.Lock()
	b.subs[ch] = struct{}{}
	b.mu.Unlock()
	go func() {
		<-ctx.Done()
		b.mu.Lock()
		delete(b.subs, ch)
		b.mu.Unlock()
		close(ch)
	}()
	return ch
}

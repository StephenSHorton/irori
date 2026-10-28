package inbox

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"time"
	"unicode/utf8"
)

func clip(s string, n int) string {
	s = strings.TrimSpace(s)
	if utf8.RuneCountInString(s) <= n {
		return s
	}
	return string([]rune(s)[:n])
}

// MaxRef caps SendOptions.Ref (runes).
const MaxRef = 200

func (b *inbox) send(ctx context.Context, from Sender, to Address, text, reason string) (Message, Delivery, error) {
	return b.sendWith(ctx, from, to, text, SendOptions{Reason: reason, Mode: ModeWake})
}

func (b *inbox) sendWith(ctx context.Context, from Sender, to Address, text string, o SendOptions) (Message, Delivery, error) {
	switch o.Mode {
	case "":
		o.Mode = ModeWake
		if b.isPerson(from) || from.isSystem() {
			o.Mode = ModeNextTurn
		}
	case ModeWake, ModeNextTurn:
	default:
		return Message{}, Delivery{}, fmt.Errorf("inbox: unknown delivery mode %q (wake or next_turn)", o.Mode)
	}
	b.mu.Lock()
	defer b.unlock()
	return b.sendLocked(ctx, from, to, text, o, nil)
}

// sendLocked validates, routes and records one message; o.Mode is already
// resolved. replyOf is the message being answered by Reply (its hop parents
// the new message when no woken turn is running).
func (b *inbox) sendLocked(ctx context.Context, from Sender, to Address, text string, o SendOptions, replyOf *Message) (Message, Delivery, error) {
	if strings.TrimSpace(from.Key) == "" {
		return Message{}, Delivery{}, fmt.Errorf("inbox: no sender; join first")
	}
	text = strings.TrimSpace(text)
	if text == "" {
		return Message{}, Delivery{}, ErrEmpty
	}
	if n := utf8.RuneCountInString(text); n > MaxText {
		return Message{}, Delivery{}, fmt.Errorf("%w: %d characters; at most %d", ErrTooLong, n, MaxText)
	}
	now := b.now()
	cards, err := b.liveCards(ctx)
	if err != nil {
		return Message{}, Delivery{}, err
	}

	// Route to exactly one recipient ("" = repo work with no desk yet).
	var recipient, repo, replyTo, forOwner string
	addr := Address{Kind: to.Kind, Value: strings.TrimSpace(to.Value)}
	switch to.Kind {
	case ToAgent:
		k, err := b.resolveAgent(from.Key, addr.Value, cards)
		if err != nil {
			return Message{}, Delivery{}, err
		}
		recipient = k
	case ToRepo:
		if addr.Value == "" {
			return Message{}, Delivery{}, fmt.Errorf("inbox: say which repo")
		}
		repo = addr.Value
		recipient = b.desk(repo, cards, now, from.Key)
	case ToReply:
		orig := b.msgs[addr.Value]
		if orig == nil {
			return Message{}, Delivery{}, fmt.Errorf("%w: message %q", ErrNotFound, addr.Value)
		}
		if orig.From.Key == from.Key {
			return Message{}, Delivery{}, ErrSelf
		}
		if orig.From.isSystem() {
			return Message{}, Delivery{}, fmt.Errorf("%w: that is Irori's own notice; it needs no reply", ErrBadState)
		}
		recipient, replyTo = orig.From.Key, orig.ID
	case ToPR:
		r, err := b.routePR(from.Key, addr.Value, cards, now)
		if err != nil {
			return Message{}, Delivery{}, err
		}
		addr.Value, recipient, repo, forOwner = r.key, r.to, r.repo, r.forOwner
	default:
		return Message{}, Delivery{}, fmt.Errorf("inbox: unknown address kind %q", to.Kind)
	}

	// Rate limit per unordered pair, both directions, sliding window.
	if !b.isPerson(from) && b.countsForRate(from.Key, recipient) {
		pk := pairKey(from.Key, recipient)
		kept := slices.DeleteFunc(b.pairs[pk], func(t time.Time) bool { return now.Sub(t) >= DirectWindow })
		b.pairs[pk] = kept
		if len(kept) >= DirectLimit {
			return Message{}, Delivery{}, fmt.Errorf("%w: %d messages between you and %s in the last %d minutes; stop the back-and-forth or ask the person",
				ErrRateLimited, len(kept), b.describe(recipient, cards), int(DirectWindow/time.Minute))
		}
	}

	m := Message{
		ID: "msg_" + b.ids.next(now), From: from, To: addr, Text: text,
		Reason: clip(o.Reason, MaxNote), ReplyTo: replyTo, Trust: Untrusted, SentAt: now,
		Ref: clip(o.Ref, MaxRef), Notice: o.Notice,
	}
	if b.isPerson(from) {
		m.Trust = UserTrust
	}
	// Hops: a message sent from inside a woken turn continues that wake's chain.
	if run, ok := b.running[from.Key]; ok {
		if parent := b.newestIn(run.WakeID); parent != nil {
			m.ParentID, m.Hop = parent.ID, parent.Hop+1
		}
	}
	if m.ParentID == "" && replyOf != nil {
		m.ParentID, m.Hop = replyOf.ID, replyOf.Hop+1
	}

	// The sender becomes addressable by the name it sent under.
	if a := normAlias(from.Name); a != "" && !b.isPerson(from) && !from.isSystem() {
		if list := b.aliases[from.Key]; len(list) == 0 || list[len(list)-1] != a {
			if err := b.emit(ctx, from.Key, KindAlias, aliasRow{Key: from.Key, Alias: a, At: now}); err != nil {
				return Message{}, Delivery{}, err
			}
		}
	}
	if err := b.emit(ctx, from.Key, KindMessageSent, m); err != nil {
		return Message{}, Delivery{}, err
	}
	r := placed(deliveryRow{DeliveryID: "dlv_" + b.ids.next(now), MessageID: m.ID, To: recipient, RepoKey: repo, Mode: o.Mode, For: forOwner}, b.hold(&m, recipient, cards))
	if err := b.putDelivery(ctx, r, from.Key); err != nil {
		return Message{}, Delivery{}, err
	}
	return *b.msgs[m.ID], b.copyOut(b.dels[r.DeliveryID]), nil
}

// newestIn is the newest message among wake's deliveries.
func (b *inbox) newestIn(wakeID string) *Message {
	w := b.wakes[wakeID]
	if w == nil {
		return nil
	}
	var best *Message
	for _, id := range w.DeliveryIDs {
		d := b.dels[id]
		if d == nil {
			continue
		}
		if m := b.msgs[d.MessageID]; m != nil && (best == nil || m.ID > best.ID) {
			best = m
		}
	}
	return best
}

// mine looks up a delivery addressed to agent.
func (b *inbox) mine(agent, deliveryID string) (*drec, error) {
	d := b.dels[deliveryID]
	if d == nil {
		return nil, fmt.Errorf("%w: delivery %q", ErrNotFound, deliveryID)
	}
	if d.To != agent || agent == "" {
		return nil, fmt.Errorf("%w: %s", ErrNotYours, deliveryID)
	}
	return d, nil
}

func badState(d *drec, what string) error {
	return fmt.Errorf("%w: can't %s a %s delivery", ErrBadState, what, d.State)
}

func (b *inbox) reply(ctx context.Context, from Sender, deliveryID, text string) (Message, Delivery, error) {
	b.mu.Lock()
	defer b.unlock()
	d, err := b.mine(from.Key, deliveryID)
	if err != nil {
		return Message{}, Delivery{}, err
	}
	switch d.State {
	// Queued counts: a reply proves the recipient has the message, which is how
	// a Grok push (never "delivered" by itself, design §4) or an inbox read
	// gets answered.
	case Queued, Delivered, Engaged, Deferred:
	default:
		return Message{}, Delivery{}, badState(d, "reply to")
	}
	orig := b.msgs[d.MessageID]
	if orig == nil {
		return Message{}, Delivery{}, fmt.Errorf("%w: message %q", ErrNotFound, d.MessageID)
	}
	m, nd, err := b.sendLocked(ctx, from, Address{Kind: ToReply, Value: orig.ID}, text, SendOptions{Mode: ModeWake}, orig)
	if err != nil {
		return Message{}, Delivery{}, err
	}
	r := d.row()
	r.State, r.Until = Answered, time.Time{}
	if err := b.putDelivery(ctx, r, from.Key); err != nil {
		return Message{}, Delivery{}, err
	}
	return m, nd, nil
}

// ack: a channel-path recipient has the delivery. Queued goes through
// Delivered to Engaged; Delivered goes to Engaged; Engaged and Answered are
// already past it.
func (b *inbox) ack(ctx context.Context, agent, deliveryID string) error {
	b.mu.Lock()
	defer b.unlock()
	d, err := b.mine(agent, deliveryID)
	if err != nil {
		return err
	}
	switch d.State {
	case Engaged, Answered:
		return nil
	case Queued:
		r := d.row()
		r.State, r.Via = Delivered, string(PathChannel)
		if err := b.putDelivery(ctx, r, agent); err != nil {
			return err
		}
	case Delivered:
	default:
		return badState(d, "ack")
	}
	r := d.row()
	r.State = Engaged
	if r.Via == "" {
		r.Via = string(PathChannel)
	}
	return b.putDelivery(ctx, b.engaged(d, r), agent)
}

// MarkRead records that agent read a queued delivery itself (a channel-path
// recipient listing its inbox): Queued becomes Delivered. Other states are
// left alone. Idempotent.
func (b *Inbox) MarkRead(ctx context.Context, agent, deliveryID string) error {
	x := b.impl
	x.mu.Lock()
	defer x.unlock()
	d, err := x.mine(agent, deliveryID)
	if err != nil {
		return err
	}
	if d.State != Queued {
		return nil
	}
	r := d.row()
	r.State, r.Via = Delivered, string(PathChannel)
	return x.putDelivery(ctx, r, agent)
}

func (b *inbox) deferDelivery(ctx context.Context, agent, deliveryID string, minutes int, note string) error {
	if minutes <= 0 {
		return fmt.Errorf("inbox: defer by a positive number of minutes")
	}
	b.mu.Lock()
	defer b.unlock()
	d, err := b.mine(agent, deliveryID)
	if err != nil {
		return err
	}
	switch d.State {
	case Queued, Delivered, Engaged, Deferred:
	default:
		return badState(d, "defer")
	}
	// Clamp before multiplying: a huge minutes would overflow the Duration.
	dur := time.Duration(min(minutes, int(MaxDefer/time.Minute))) * time.Minute
	r := d.row()
	r.State, r.Until, r.Note, r.WakeID = Deferred, b.now().Add(dur), clip(note, MaxNote), ""
	r.EngagedOnce = true
	r.Reminded = false // the new time earns its own reminder
	return b.putDelivery(ctx, r, agent)
}

func (b *inbox) skip(ctx context.Context, agent, deliveryID, note string) error {
	b.mu.Lock()
	defer b.unlock()
	d, err := b.mine(agent, deliveryID)
	if err != nil {
		return err
	}
	switch d.State {
	case Skipped:
		return nil
	case Queued, Delivered, Engaged, Deferred:
	default:
		return badState(d, "skip")
	}
	r := d.row()
	r.State, r.Note, r.Until = Skipped, clip(note, MaxNote), time.Time{}
	return b.putDelivery(ctx, r, agent)
}

// release is the person's Deliver: a held delivery is queued past the
// policy. Unowned repo work goes to the front desk if there is one now.
func (b *inbox) release(ctx context.Context, deliveryID, by string) error {
	b.mu.Lock()
	defer b.unlock()
	d := b.dels[deliveryID]
	if d == nil {
		return fmt.Errorf("%w: delivery %q", ErrNotFound, deliveryID)
	}
	if d.State != Held {
		return badState(d, "release")
	}
	r := d.row()
	if r.To == "" {
		cards, err := b.liveCards(ctx)
		if err != nil {
			return err
		}
		sender := ""
		if m := b.msgs[d.MessageID]; m != nil {
			sender = m.From.Key
		}
		r.To = b.desk(d.RepoKey, cards, b.now(), sender) // never back to its sender
		if r.To == "" {
			return fmt.Errorf("%w: no agent in that repo can take it yet", ErrBadState)
		}
	}
	r.State, r.Hold, r.WakeID = Queued, "", ""
	return b.putDelivery(ctx, r, by)
}

func (b *inbox) dismiss(ctx context.Context, deliveryID, by string) error {
	b.mu.Lock()
	defer b.unlock()
	d := b.dels[deliveryID]
	if d == nil {
		return fmt.Errorf("%w: delivery %q", ErrNotFound, deliveryID)
	}
	if d.State == Dismissed {
		return nil
	}
	if d.State != Held {
		return badState(d, "dismiss")
	}
	r := d.row()
	r.State = Dismissed
	return b.putDelivery(ctx, r, by)
}

// newestFirst orders deliveries by creation, newest first.
func (b *inbox) newestFirst(keep func(*drec) bool, limit int) []Delivery {
	if limit <= 0 {
		limit = DefaultInboxList
	}
	out := []Delivery{}
	for i := len(b.order) - 1; i >= 0 && len(out) < limit; i-- {
		if d := b.dels[b.order[i]]; keep(d) {
			out = append(out, b.copyOut(d))
		}
	}
	return out
}

// forAgent is the recipient's own view: what a hold kept from it (held,
// or dismissed by the person) never shows, text included.
func (b *inbox) forAgent(agent string, all bool, limit int) []Delivery {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.newestFirst(func(d *drec) bool {
		if d.To != agent || d.State == Held || d.State == Dismissed {
			return false
		}
		return all || d.State.Open()
	}, limit)
}

func (b *inbox) list(limit int) []Delivery {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.newestFirst(func(*drec) bool { return true }, limit)
}

func (b *inbox) openList() []Delivery {
	b.mu.Lock()
	defer b.mu.Unlock()
	out := make([]Delivery, 0, len(b.open))
	for _, d := range b.open {
		out = append(out, b.copyOut(d))
	}
	return out
}

func (b *inbox) get(deliveryID string) (Delivery, bool) {
	b.mu.Lock()
	defer b.mu.Unlock()
	d := b.dels[deliveryID]
	if d == nil {
		return Delivery{}, false
	}
	return b.copyOut(d), true
}

// Message returns one message by id.
func (b *Inbox) Message(id string) (Message, bool) {
	x := b.impl
	x.mu.Lock()
	defer x.mu.Unlock()
	m := x.msgs[id]
	if m == nil {
		return Message{}, false
	}
	return *m, true
}

// Aliases returns the names agent is addressable by besides its card name,
// newest last.
func (b *Inbox) Aliases(agent string) []string {
	x := b.impl
	x.mu.Lock()
	defer x.mu.Unlock()
	return slices.Clone(x.aliases[agent])
}

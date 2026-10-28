// Package inbox is Irori's messaging core: messages, the per-recipient
// delivery state machine, wake planning, policy, limits and the sweeper.
// The spec is docs/design/v0.2-talk.md §2–§5; the port references are
// docs/research/v02/router.md (§6, §7, §10) and mod.md (§4, §5, §11).
//
// State lives in the event log (kinds.go) and is rebuilt into memory by
// replaying it at New; every mutation appends a row first and then updates
// the fold, so a crash never loses an accepted message. Nothing here knows
// about HTTP, Wails or the mod: transports call these methods and listen on
// Subscribe.
//
// CONTRACT (implemented by the inbox work package; keep these signatures):
package inbox

import (
	"context"
	"errors"
	"time"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// Errors callers branch on. Wrap them with detail (fmt.Errorf("%w: ...")).
var (
	ErrUnknownAgent  = errors.New("inbox: no agent by that name or key")
	ErrAmbiguous     = errors.New("inbox: that name matches more than one agent")
	ErrRateLimited   = errors.New("inbox: too many messages between these two agents")
	ErrNotYours      = errors.New("inbox: that delivery isn't addressed to you")
	ErrNotFound      = errors.New("inbox: no such delivery or message")
	ErrBadState      = errors.New("inbox: not allowed in the delivery's current state")
	ErrTooLong       = errors.New("inbox: message text is too long")
	ErrUnimplemented = errors.New("inbox: not implemented yet")
	// ErrNoCard: Options.OnRecipientGone found no card for the message here
	// (its ref names none, or the task isn't on this device). The message
	// then goes to the person instead.
	ErrNoCard = errors.New("inbox: the message names no card here")
)

// NoteRecipientGone is the note on a delivery the sweeper took away from a
// session that ended (Options.OnRecipientGone). Handed to a card, the note
// is "recipient gone; on task:<id>".
const NoteRecipientGone = "recipient gone"

// Limits and timings (design §4–§5). Tests may override through Options.
const (
	MaxText          = 8000
	MaxWakeBytes     = 16 << 10
	DirectLimit      = 8
	DirectWindow     = 10 * time.Minute
	MaxHop           = 6
	UnengagedLimit   = 10 * time.Minute
	GoneGrace        = 3 * time.Minute
	QuietLimit       = 60 * time.Minute
	MaxDefer         = 12 * time.Hour
	ReattachGrace    = 2 * time.Minute
	MaxAliases       = 10
	MaxAliasLen      = 60
	MinKeyPrefixLen  = 8
	SweepInterval    = 60 * time.Second
	DefaultInboxList = 50
)

// Sender is who is sending, as the Core established it (never self-claimed).
type Sender struct {
	Key  string `json:"key"`  // agent key "<device>/<session>", or "user:<device>" for the GUI
	Name string `json:"name"` // display name; also recorded as an alias for Key
	Kind string `json:"kind"` // "claude", "grok", "user"
}

// IsUser reports whether the sender is a person at the GUI.
func (s Sender) IsUser() bool { return s.Kind == "user" }

// AddressKind says how To is resolved.
type AddressKind string

const (
	ToAgent AddressKind = "agent" // key, key prefix (>= MinKeyPrefixLen), name or alias
	ToRepo  AddressKind = "repo"  // repo_key; goes to that repo's front desk
	ToReply AddressKind = "reply" // a message id; goes to that message's sender
	// ToPR is a ledger key "pr:<owner/name>#<n>" (v0.3, design v0.3 §6): the
	// best-ranked live owner from Options.PROwners, else the front desk of
	// repo "remote:github.com/<owner/name>" (the delivery note names the owner
	// it would have gone to), else queued unowned like a repo with no desk.
	ToPR AddressKind = "pr"
)

// Address is a message's target before routing.
type Address struct {
	Kind  AddressKind `json:"kind"`
	Value string      `json:"value"`
}

// Trust is how a recipient should weigh the text.
type Trust string

const (
	Untrusted Trust = "untrusted" // from an agent
	UserTrust Trust = "user"      // typed by the person in the GUI
)

// Message is one sent message (the message.sent payload).
type Message struct {
	ID       string    `json:"id"` // minted by the inbox; sortable
	From     Sender    `json:"from"`
	To       Address   `json:"to"`
	Text     string    `json:"text"`
	Reason   string    `json:"reason,omitempty"`
	ReplyTo  string    `json:"replyTo,omitempty"`
	ParentID string    `json:"parentId,omitempty"`
	Hop      int       `json:"hop"`
	Trust    Trust     `json:"trust"`
	SentAt   time.Time `json:"sentAt"`
	// Ref is what the message is about, from SendOptions.Ref ("task:<id>",
	// "comment:<id>", ...). The fence prints it as ref="..." so the
	// recipient can show it.
	Ref string `json:"ref,omitempty"`
	// Notice is SendOptions.Notice: a Board notice that needs no reply.
	Notice bool `json:"notice,omitempty"`
	// Attachments are files on the message (v0.4 §6.6): refs to the
	// attaching device's store, fetched on demand with attachment_get.
	Attachments []blob.FileRef `json:"attachments,omitempty"`
}

// State is a delivery's state (design §4).
type State string

const (
	Queued    State = "queued"
	Held      State = "held"
	Delivered State = "delivered"
	Engaged   State = "engaged"
	Answered  State = "answered"
	Deferred  State = "deferred"
	Skipped   State = "skipped"
	Rejected  State = "rejected"
	Dismissed State = "dismissed"
	Moved     State = "moved" // terminal for this delivery; a new one exists for the new recipient
)

// Open reports whether s still needs something to happen.
func (s State) Open() bool {
	switch s {
	case Queued, Held, Delivered, Engaged, Deferred:
		return true
	}
	return false
}

// HoldReason says why a delivery is held.
type HoldReason string

const (
	HoldBlocked        HoldReason = "blocked"
	HoldNotAllowed     HoldReason = "not_allowed"
	HoldBypassReceiver HoldReason = "bypass_receiver"
	HoldPaused         HoldReason = "paused"
	HoldHopLimit       HoldReason = "hop_limit"
	HoldUnowned        HoldReason = "unowned" // repo message with no live desk
)

// DeliveryMode is how a delivery reaches a Claude session (decision 18,
// spike 04). It is per delivery and fixed at send.
type DeliveryMode string

const (
	// ModeWake submits a prompt: a full turn starts now (design v0.2 §4).
	ModeWake DeliveryMode = "wake"
	// ModeNextTurn never starts a turn. On the mod path it goes out as an
	// append (NextAppend: $.session.append, a meta user row the model reads
	// on its next request); it also rides along in any wake that goes out
	// first. On the channel path (Grok, which can't append silently) it is
	// pushed like a wake, with meta mode=next_turn. Rules:
	//   - delivered = the append resolved (MarkDelivered with its token);
	//   - engaged = the next main-loop turn.start of that session after
	//     delivery, whoever started it (the mod reports MarkEngaged with the
	//     append token); an append made mid-turn may be read sooner, which is
	//     fine; an engaged append sets no running wake (hop stays 0: the
	//     person, not the message, started that turn);
	//   - the sweeper never moves a next_turn delivery for being unengaged
	//     (UnengagedLimit) and never sends it a reminder (QuietLimit); the
	//     gone-move for repo work still applies;
	//   - nothing about next_turn ever wakes.
	ModeNextTurn DeliveryMode = "next_turn"
)

// Delivery is one message's journey to one recipient.
type Delivery struct {
	ID        string       `json:"id"`
	MessageID string       `json:"messageId"`
	To        string       `json:"to"` // agent key; "" while a repo message has no desk
	RepoKey   string       `json:"repoKey,omitempty"`
	Mode      DeliveryMode `json:"mode,omitempty"` // "" = ModeWake (old rows)
	State     State        `json:"state"`
	Hold      HoldReason   `json:"hold,omitempty"`
	Via       string       `json:"via,omitempty"`    // "mod", "channel", "gui"
	WakeID    string       `json:"wakeId,omitempty"` // the wake that carried it
	Note      string       `json:"note,omitempty"`   // defer/skip note
	For       string       `json:"for,omitempty"`    // PR work its owner couldn't take: that owner, "name (key)"; or "recipient gone: name (key)"
	Until     time.Time    `json:"until,omitempty"`  // deferral due time
	MovedTo   string       `json:"movedTo,omitempty"`
	Reminded  bool         `json:"reminded"`
	CreatedAt time.Time    `json:"createdAt"`
	UpdatedAt time.Time    `json:"updatedAt"`
	Message   Message      `json:"message"` // denormalized for callers
}

// Wake is one prompt to submit to a recipient, or (Mode ModeNextTurn) one
// append. Prompt is the full text (design §5 fencing), and contains Token
// exactly once.
//
// Batching: everything queued for the recipient goes out together, up to
// MaxWakeBytes. A wake carries its wake items first, then every queued
// next_turn item (each fenced with mode="next_turn"). An append carries
// only next_turn items. What doesn't fit is still handed over, as Overflow:
// those ids are listed at the end of Prompt in one line ("N more waiting:
// read them with the irori inbox tool: <ids>") rather than in full, and are
// marked delivered and engaged with the rest.
type Wake struct {
	ID string `json:"id"`
	To string `json:"to"`
	// Token is "[irori-wake:<id>]", "[irori-reminder:<id>]", or for an
	// append "[irori-append:<id>]" (escapeText neutralizes all three forms
	// in message text).
	Token       string       `json:"token"`
	Reminder    bool         `json:"reminder"`
	Mode        DeliveryMode `json:"mode,omitempty"` // ModeNextTurn: an append (mod) or a next_turn-only push (channel)
	DeliveryIDs []string     `json:"deliveryIds"`    // includes Overflow
	Overflow    []string     `json:"overflow,omitempty"`
	Prompt      string       `json:"prompt"`
}

// SendOptions are SendWith's choices beyond the text.
type SendOptions struct {
	Reason string
	// Mode "" picks the default: ModeNextTurn when the sender is a person
	// (IsUser) or SystemSender, ModeWake for an agent. Board comments and
	// assignments pass ModeNextTurn unless the author chose Wake.
	Mode DeliveryMode
	// Ref is what the message is about (a search ref, e.g. "task:<id>").
	Ref string
	// Notice marks a Board notice (coord.Notice): it needs no reply, so it
	// closes as Skipped (note "notice") once engaged, and the fence says no
	// reply is needed.
	Notice bool
	// Attachments are files the Core already checked and stored (v0.4
	// §6.6), at most MaxAttachments.
	Attachments []blob.FileRef
}

// SystemKey is the sender key for notices Irori itself sends (a blocker
// that resolved by itself). SystemSender is exempt from the rate limit, and
// like a person it passes the bypass-receiver check and allowlist mode
// (otherwise every notice to a bypass session would be held); Pause all and
// the block list still apply. It arrives as Untrusted (its text quotes
// agent-written titles).
const SystemKey = "irori"

// SystemSender is Irori's own sender.
func SystemSender() Sender { return Sender{Key: SystemKey, Name: "Irori", Kind: "irori"} }

// isSystem reports whether s is SystemSender (the key is the identity; the
// Core establishes senders, nobody claims one).
func (s Sender) isSystem() bool { return s.Key == SystemKey }

// Mode is the wake policy mode.
type Mode string

const (
	Everyone  Mode = "everyone"
	Allowlist Mode = "allowlist"
)

// Policy is the per-device wake policy. Block/Allow hold sender keys or names.
type Policy struct {
	Mode   Mode     `json:"mode"`
	Block  []string `json:"block"`
	Allow  []string `json:"allow"`
	Paused bool     `json:"paused"`
}

// Path is a wake path an agent can be reached by.
type Path string

const (
	PathMod     Path = "mod"     // an `irori attach` stream from the Irori mod
	PathChannel Path = "channel" // a Grok member's bound `irori mcp` process
)

// ChangeKind classifies a Subscribe notification.
type ChangeKind string

const (
	WakeReady ChangeKind = "wake_ready" // To has something to submit: call NextWake
	// AppendReady: To has next_turn items and no append outstanding: a mod
	// stream that negotiated appends (attach.Version >= 2) calls NextAppend.
	AppendReady ChangeKind = "append_ready"
	HeldNew     ChangeKind = "held"    // a delivery to To was held
	MovedAway   ChangeKind = "moved"   // a delivery left To
	Updated     ChangeKind = "updated" // anything else; UIs refresh
)

// Change is a notification for transports and the UI.
type Change struct {
	Kind       ChangeKind `json:"kind"`
	To         string     `json:"to,omitempty"`
	DeliveryID string     `json:"deliveryId,omitempty"`
}

// Roster is the presence the inbox routes against (core.Core / presence.Watcher).
type Roster interface {
	Agents() []presence.AgentCard
}

// Options configure New.
type Options struct {
	DeviceID string
	Now      func() time.Time // default time.Now
	// PROwners lists a PR's owner agent keys, best first (capture's ledger).
	// Nil makes every ToPR send fall through to the repo's front desk.
	PROwners func(prKey string) []string
	// IsOwner reports whether a "user:<device>" key is the person's (v0.4
	// identity.Chain.IsOwner). Nil accepts exactly "user:<DeviceID>".
	IsOwner func(key string) bool
	// OnRecipientGone puts a message whose recipient (to) has ended on the
	// card its Ref names (task: or comment:), returning the task id. The
	// sweeper calls it without the inbox lock held, then closes the delivery
	// as Skipped. ErrNoCard sends the message to the person instead; any
	// other error leaves it for the next sweep. Nil sends card messages to
	// the person too.
	OnRecipientGone func(ctx context.Context, m Message, to string) (taskID string, err error)
	// Overrides for tests; zero means the constant above.
	UnengagedLimit, GoneGrace, QuietLimit, ReattachGrace time.Duration
}

// Inbox is the messaging core. Safe for concurrent use.
type Inbox struct {
	impl *inbox
}

// New replays the log into memory and returns a ready inbox.
func New(ctx context.Context, st *store.Store, roster Roster, opts Options) (*Inbox, error) {
	return newInbox(ctx, st, roster, opts)
}

// Send routes and records a message and its delivery. Policy, rate limit and
// hop limit apply (design §5); a held delivery is still a successful send.
// GUI senders (IsUser) are exempt from the rate limit and arrive as UserTrust.
// Send is the v0.2 form and always uses ModeWake; new callers use SendWith.
func (b *Inbox) Send(ctx context.Context, from Sender, to Address, text, reason string) (Message, Delivery, error) {
	return b.impl.send(ctx, from, to, text, reason)
}

// SendWith is Send with options: the delivery mode (default by sender, see
// SendOptions.Mode), a ref and the notice flag. Send(…, reason) must equal
// SendWith(…, SendOptions{Reason: reason, Mode: ModeWake}).
func (b *Inbox) SendWith(ctx context.Context, from Sender, to Address, text string, o SendOptions) (Message, Delivery, error) {
	return b.impl.sendWith(ctx, from, to, text, o)
}

// NextAppend returns the append to make to agent now, or nil when it has no
// queued next_turn items, when Pause all is on, or while an earlier append
// to it is issued but not yet delivered. Only the mod path appends; the
// channel path gets next_turn items from NextWake. The deliveries are
// recorded as handed to it (a wake.issued row with mode next_turn), and an
// append not confirmed delivered within ReattachGrace is reclaimed like a
// wake.
func (b *Inbox) NextAppend(ctx context.Context, agent string) (*Wake, error) {
	return b.impl.nextAppend(ctx, agent)
}

// OutstandingAppend is the append issued to agent and not yet confirmed
// delivered, rebuilt with its token (a reattached stream sends it again, and
// the mod dedupes on the token), or nil.
func (b *Inbox) OutstandingAppend(agent string) *Wake { return b.impl.outstandingAppend(agent) }

// Reply answers a delivery addressed to from: marks it Answered and sends a
// ToReply message back to its sender.
func (b *Inbox) Reply(ctx context.Context, from Sender, deliveryID, text string) (Message, Delivery, error) {
	return b.impl.reply(ctx, from, deliveryID, text)
}

// SetReachable records that agent can (on) or can no longer (off) be woken by
// path. Attach streams and bound Grok members call it. The first attach time
// orders the front desk; a mod path that drops keeps counting as reachable for
// ReattachGrace.
func (b *Inbox) SetReachable(agent string, path Path, on bool) { b.impl.setReachable(agent, path, on) }

// NextWake returns the wake to submit to agent now, or nil when there is
// nothing queued or a previous wake is still outstanding (delivered, not yet
// engaged). The returned deliveries are recorded as handed to that wake.
// On the mod path only queued ModeWake items make a wake; queued next_turn
// items then ride along (see Wake). On the channel path next_turn items make
// a push too (Mode ModeNextTurn when it holds nothing else).
func (b *Inbox) NextWake(ctx context.Context, agent string, path Path) (*Wake, error) {
	return b.impl.nextWake(ctx, agent, path)
}

// MarkDelivered: the transport handed the wake over ($.prompt.submit resolved),
// or an append's $.session.append resolved (append token). Idempotent. Not
// called for channel pushes (see Ack).
func (b *Inbox) MarkDelivered(ctx context.Context, agent, token string) error {
	return b.impl.markDelivered(ctx, agent, token)
}

// MarkEngaged: a turn whose text contains token started. Reminder tokens are
// accepted and ignored. Sets agent's running wake (for hop counting).
// With an append token it means "the first main-loop turn after the append
// was delivered started": its deliveries go Engaged (Notice ones then
// Skipped, note "notice"), and no running wake is set. TurnEnded ignores
// append tokens.
func (b *Inbox) MarkEngaged(ctx context.Context, agent, token, turnID string) error {
	return b.impl.markEngaged(ctx, agent, token, turnID)
}

// TurnEnded: that turn finished. Clears the running wake and may make the
// next wake ready.
func (b *Inbox) TurnEnded(ctx context.Context, agent, token, turnID string) error {
	return b.impl.turnEnded(ctx, agent, token, turnID)
}

// Ack: a channel-path recipient confirmed it has the delivery (Delivered then Engaged).
func (b *Inbox) Ack(ctx context.Context, agent, deliveryID string) error {
	return b.impl.ack(ctx, agent, deliveryID)
}

// Defer postpones a delivery addressed to agent; minutes is clamped to MaxDefer.
func (b *Inbox) Defer(ctx context.Context, agent, deliveryID string, minutes int, note string) error {
	return b.impl.deferDelivery(ctx, agent, deliveryID, minutes, note)
}

// Skip closes a delivery addressed to agent without a reply.
func (b *Inbox) Skip(ctx context.Context, agent, deliveryID, note string) error {
	return b.impl.skip(ctx, agent, deliveryID, note)
}

// Release is the person's Deliver on a held delivery: it re-queues it past the
// policy. by is "user:<device>".
func (b *Inbox) Release(ctx context.Context, deliveryID, by string) error {
	return b.impl.release(ctx, deliveryID, by)
}

// Dismiss is the person's Dismiss on a held delivery.
func (b *Inbox) Dismiss(ctx context.Context, deliveryID, by string) error {
	return b.impl.dismiss(ctx, deliveryID, by)
}

// For lists deliveries addressed to agent: open ones, or all recent when all.
// It is the recipient's own view, so deliveries held from it (or dismissed
// while held) are left out; only List shows them.
func (b *Inbox) For(agent string, all bool, limit int) []Delivery {
	return b.impl.forAgent(agent, all, limit)
}

// List lists recent deliveries across every recipient, newest first (GUI).
func (b *Inbox) List(limit int) []Delivery { return b.impl.list(limit) }

// Open lists every open delivery (State.Open), held ones included, in no
// particular order. Subscribe drops changes for a slow reader; a reader
// that keeps state per delivery re-reads with this.
func (b *Inbox) Open() []Delivery { return b.impl.openList() }

// Get returns one delivery.
func (b *Inbox) Get(deliveryID string) (Delivery, bool) { return b.impl.get(deliveryID) }

// Policy returns the current policy.
func (b *Inbox) Policy() Policy { return b.impl.policy() }

// SetPolicy replaces the policy (GUI or person-typed /irori only; callers
// enforce that). Unpausing re-queues HoldPaused deliveries.
func (b *Inbox) SetPolicy(ctx context.Context, p Policy, by string) error {
	return b.impl.setPolicy(ctx, p, by)
}

// Sweep runs the sweeper once (design §4): reassign, gone-moves, reminders,
// due deferrals, front-desk succession for unowned repo work, and agent work
// whose recipient has ended handed to its card or the person. Core calls it
// every SweepInterval.
func (b *Inbox) Sweep(ctx context.Context) error {
	hs, err := b.impl.sweep(ctx)
	if err != nil {
		return err
	}
	return b.impl.handOff(ctx, hs)
}

// EndedFor is how long agent has been out of presence, as the sweeper last
// saw it while it had open work here (0 while it is around, or before a
// sweep noticed). Past GoneGrace its agent work is handed on.
func (b *Inbox) EndedFor(agent string) time.Duration {
	x := b.impl
	x.mu.Lock()
	defer x.mu.Unlock()
	return x.endedFor(agent, x.now())
}

// Subscribe delivers changes until ctx ends (buffered; slow readers drop and
// should re-read).
func (b *Inbox) Subscribe(ctx context.Context) <-chan Change { return b.impl.subscribe(ctx) }

// FencePrompt builds the prompt for a wake from its messages (exported for
// the mod/channel transports' tests). See design §5.
func FencePrompt(token string, reminder bool, msgs []Delivery) string {
	return fencePrompt(token, reminder, msgs)
}

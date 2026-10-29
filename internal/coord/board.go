package coord

// The Board (docs/design/board.md, decisions 16-25): blocked as a state,
// shared tasks, comments, assignment notices, links and refused leases.
//
// CONTRACT (implemented by the coord work package, A1; keep these
// signatures). The Board is a scratch pad, not a tracker: there is no field
// for priority, estimate, label, due date or backlog, and no retention
// timer: a card stays until it is done or closed, and Board() only
// collapses the Done group.
//
// coord never imports inbox. Every call that someone should hear about
// returns Notices; the caller (the tools layer, the GUI's BoardService, or
// Core for SweepNotices) routes each one through inbox.SendWith (see Notice).

import (
	"context"
	"time"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/search"
)

// Board limits (board.md "Instructions for agents"). A title stays at
// maxTitle (200). The v0.2 objective cap (maxObjective, 8000) is wider than
// board.md's 4000; new Board inputs (comments, reasons) use the board.md
// caps, and the objective cap is left as it is.
const (
	MaxComment      = 4000 // runes in a comment
	MaxAttachments  = 10   // files on one comment (v0.4 §6.6)
	MaxReason       = 500  // runes in a block reason or unblock note
	MaxRef          = 300  // runes in BlockedBy.Ref
	MaxKey          = 200  // runes in an assignee's or joiner's key
	DefaultDoneCard = 20   // Done cards Board() returns by default
	MaxDoneCard     = 200
)

// BlockerKind says what a blocked task waits on.
type BlockerKind string

const (
	BlockedOnLease    BlockerKind = "lease"    // Ref: a resource (normalized like Acquire)
	BlockedOnTask     BlockerKind = "task"     // Ref: a task id
	BlockedOnPerson   BlockerKind = "person"   // Ref: a "user:*" key, or "" for whoever is at the app
	BlockedOnAgent    BlockerKind = "agent"    // Ref: an agent key
	BlockedOnExternal BlockerKind = "external" // Ref: free text or a URL (CI, a review, a vendor)
)

// BlockedBy names what is in the way.
type BlockedBy struct {
	Kind BlockerKind `json:"kind"`
	Ref  string      `json:"ref,omitempty"`
}

// Comment is one task.comment row. Comments are their own rows, never
// folded into the task (Task.Comments is only a count).
type Comment struct {
	ID     string    `json:"id"` // newID, like task ids; ref "comment:<id>"
	TaskID string    `json:"taskId"`
	By     string    `json:"by"` // agent key or "user:*"
	Text   string    `json:"text"`
	Wake   bool      `json:"wake"` // the author asked to wake the holders (decision 18)
	At     time.Time `json:"at"`
	// Attachments are files on the comment (v0.4 §6.6), fetched on demand
	// with the attachment_get tool.
	Attachments []blob.FileRef `json:"attachments,omitempty"`
	// A mirrored message (RecordMessage) has MsgID, its inbox message id
	// (also its ID), and To, who it was sent to. Gone: the recipient had
	// ended and Irori left it on the card. Held: it hasn't been let through
	// the wake policy yet (held, or its recipient's device hasn't decided),
	// so Text is empty until a fill row brings it. Files counts the
	// message's attachments; they stay on the message.
	To    string `json:"to,omitempty"`
	MsgID string `json:"msgId,omitempty"`
	Gone  bool   `json:"gone,omitempty"`
	Held  bool   `json:"held,omitempty"`
	Files int    `json:"files,omitempty"`
	// BlockerID: the comment is on that blocker item. The person's answer
	// (To is the raiser it went to, Pick the 1-based option picked, 0 for
	// their own words) or an agent's reply (Missing: what it still needs).
	BlockerID string   `json:"blockerId,omitempty"`
	Pick      int      `json:"pick,omitempty"`
	Missing   []string `json:"missing,omitempty"`
	// Device wrote the row (from the log, not the payload). HLC and Seq are
	// its position, (HLC, Device, Seq); they never go on the wire.
	Device string        `json:"device"`
	HLC    hlc.Timestamp `json:"-"`
	Seq    int64         `json:"-"`
	// primary: a mirrored message's own row (not gone, not fill) has been
	// folded; until then a gone or fill row stands in for it. passed is
	// the text a gone or fill row brought: the message was let through, so
	// it shows even when the primary row was written held.
	primary bool
	passed  string
	// person: the person wrote it (signed actor "user" and by an owner
	// key); the payload's by alone never makes a blocker answer.
	person bool
}

// MessageEntry is an inbox message about a card, for RecordMessage. From
// and To are the sender's and recipient's keys; Writer "irori" writes the
// row as Irori itself (the recipient-gone fallback, a fill), else as From.
// Fill: the recipient's device let the message through its wake policy,
// so an entry recorded Held gets its text.
type MessageEntry struct {
	TaskID, MsgID, From, To, Text string
	Files                         int
	Held, Gone, Fill              bool
	Writer                        string
}

// NoticeKind says why a Notice exists.
type NoticeKind string

const (
	NoticeAssigned  NoticeKind = "assigned"  // you were assigned a task (decision 21)
	NoticeComment   NoticeKind = "comment"   // someone commented on a task you hold
	NoticeUnblocked NoticeKind = "unblocked" // a task you hold is no longer blocked
)

// Notice is something coord wants an agent to hear. coord only builds it;
// the caller routes it, once per recipient:
//
//	inbox.SendWith(ctx, sender, inbox.Address{Kind: inbox.ToAgent, Value: to}, n.Text,
//	    inbox.SendOptions{Mode: next_turn unless n.Wake, Ref: n.Ref, Notice: true})
//
// where sender is the caller's own inbox.Sender (the person, or the agent
// that commented or assigned), or inbox.SystemSender for SweepNotices.
// Recipients that are people ("user:*") are never in To: the Board itself is
// how a person hears.
type Notice struct {
	Kind   NoticeKind `json:"kind"`
	TaskID string     `json:"taskId"`
	Ref    string     `json:"ref"` // "task:<id>", or "comment:<id>" for a comment
	Title  string     `json:"title"`
	// To are agent keys: for assigned, the assignee; for comment and
	// unblocked, the task's agent holders minus By (no holders: its creator,
	// if that is an agent other than By). Empty means nobody to tell.
	To []string `json:"to,omitempty"`
	By string   `json:"by"`
	// Text is the message body, built by coord, plain and short: what
	// happened, the task's title and ref, and for assigned its objective and
	// done criteria (each cut to 600 runes), for comment the comment text.
	// It ends with how to go deeper ("show task:<id> for the card, its
	// comments and links").
	Text string `json:"text"`
	Wake bool   `json:"wake"` // only a comment's own Wake sets it
	// Attachments are a comment's files; the message carries them too.
	Attachments []blob.FileRef `json:"attachments,omitempty"`
}

// Refusal is one lease.refused row: by asked for resource and heldBy had it.
// It is active while heldBy still holds a lease that conflicts with
// resource and by hasn't since acquired it; that is derived on read, with no
// timer. Acquire writes at most one row per (resource, by, heldBy) while an
// earlier one is active.
type Refusal struct {
	Resource string    `json:"resource"` // what by asked for (normalized)
	By       string    `json:"by"`
	HeldBy   string    `json:"heldBy"`
	Held     string    `json:"held"`  // the conflicting lease's resource (may differ: overlapping globs)
	Until    time.Time `json:"until"` // the conflicting lease's Until when refused
	At       time.Time `json:"at"`
	// Conflict marks by's own lease given up after a partition (v0.4
	// §6.4): another device's lease on the resource stood.
	Conflict bool `json:"conflict,omitempty"`
}

// BoardFilter narrows Board.
type BoardFilter struct {
	RepoKey   string // "" = every repo, plus tasks with no repo
	For       string // the viewer ("user:*" or an agent key): who "needs you" means
	DoneLimit int    // Done cards to return; 0 = DefaultDoneCard, max MaxDoneCard
}

// Card is one task on the Board.
type Card struct {
	Task
	LastComment *Comment `json:"lastComment,omitempty"`
	// Resolved: the task is Blocked and everything its open items wait on
	// has gone away (the lease is no longer held, the task is terminal).
	// SweepNotices closes such blockers; Resolved shows the gap until it runs.
	Resolved bool `json:"resolved,omitempty"`
	// Gone: when only some open items' leases or tasks went away, their ids
	// (the sweep closes just those; the others, a question, still wait).
	Gone []string `json:"gone,omitempty"`
	// Overlaps, on a Shared task, lists pairs "<resource> ~ <resource>" of
	// its holders' leases that overlap (decision 20's warning).
	Overlaps []string `json:"overlaps,omitempty"`
}

// LeaseCard is a lease shown on the Board because it blocks something: a
// blocked task names it (BlockedOnLease) or a refusal is active against it.
// It goes away when the lease is released or expires.
type LeaseCard struct {
	Lease
	Refused []Refusal `json:"refused,omitempty"`
	Blocks  []string  `json:"blocks,omitempty"` // ids of tasks blocked on it
}

// BoardView is the Board, grouped for the GUI. Each task is in exactly one
// group, picked in this order:
//
//  1. Done: terminal states, newest UpdatedAt first, at most DoneLimit
//     (DoneTotal counts them all). Collapsed in the GUI; never timed out.
//  2. NeedsYou: InputRequired; Blocked on a person (Ref "" or For); or
//     Holders contains For (when For is a person). For "" is whoever is at
//     the app: every person blocker counts. For an agent viewer, NeedsYou
//     holds tasks assigned to it that it hasn't taken up yet (For is in
//     Assigned; not while Blocked) and tasks Blocked on it
//     (BlockedOnAgent, Ref For).
//  3. Blocked: every other Blocked task.
//  4. Working: has a holder (one working it, or an assignee that hasn't
//     claimed yet).
//  5. Open: no holder.
//
// Groups 2-5 are newest UpdatedAt first. Leases are LeaseCards, soonest
// expiry first; the GUI shows them with Blocked.
type BoardView struct {
	NeedsYou  []Card      `json:"needsYou"`
	Blocked   []Card      `json:"blocked"`
	Working   []Card      `json:"working"`
	Open      []Card      `json:"open"`
	Done      []Card      `json:"done"`
	DoneTotal int         `json:"doneTotal"`
	Leases    []LeaseCard `json:"leases"`
}

// Assign gives task id to `to` (an agent key or a person's "user:*" key).
// Any agent may assign (decision 21): by may be the creator, a holder, a
// person, or anyone when the task has no holder. On a task that isn't
// Shared, to replaces the holder; on a Shared task, to is added. Either way
// to is in Assigned until it claims, joins or posts the task's state, so an
// assignment nobody picked up shows. to is an agent key or "user:*" (one
// line, at most MaxKey runes, never "irori"; else ErrInvalid). Assigning
// to the current (or an existing) holder is a no-op with no notice. The
// notice is NoticeAssigned to `to` (no recipients when to is a person, or
// when by assigned itself).
func (c *Coord) Assign(ctx context.Context, by, id, to string) (Task, Notice, error) {
	return c.impl.assignNotice(ctx, by, id, to)
}

// Join adds who to a Shared task's holders (by is who, or a person adding
// someone; who is checked like Assign's to). An assignee joining takes the
// task up: it leaves Assigned and the row is written. ErrNotAllowed on a task that isn't Shared: claim it instead.
// Joining moves a Submitted task to Working.
func (c *Coord) Join(ctx context.Context, by, id, who string) (Task, error) {
	return c.impl.join(ctx, by, id, who)
}

// Leave removes who from the task's holders (by is who, or a person). The
// state is left alone; a task with no holders shows as Open (or in its
// Blocked/NeedsYou group). ErrNotFound when who doesn't hold it.
func (c *Coord) Leave(ctx context.Context, by, id, who string) (Task, error) {
	return c.impl.leave(ctx, by, id, who)
}

// SetShared turns sharing on or off (the creator, a holder or a person).
// Turning it off while more than one holds the task is ErrInvalid.
func (c *Coord) SetShared(ctx context.Context, by, id string, shared bool) (Task, error) {
	return c.impl.setShared(ctx, by, id, shared)
}

// Block moves an open task to Blocked with a reason (required, at most
// MaxReason runes) and optionally what blocks it. The holder(s), the
// creator or a person may. BlockedBy is checked: a lease ref must parse as a
// resource that someone holds now (else ErrInvalid: SweepNotices would close
// it at once), a task ref must name another task that isn't terminal (same
// reason), a person ref must be "" or
// "user:*" (an agent ref must be non-empty; an external one may be empty).
// Blocking a blocked task again replaces the reason. No notice.
//
// Since the cards (cards.go), Block is Raise with the reason as the
// question (a reason longer than MaxQuestion goes whole into the context):
// it makes one blocker item, and blocking again through Block replaces that
// item rather than adding one. Items raised with Raise stay.
func (c *Coord) Block(ctx context.Context, by, id, reason string, blockedBy *BlockedBy) (Task, error) {
	return c.impl.block(ctx, by, id, reason, blockedBy)
}

// Unblock closes a task's blocker: anyone, agent or person, may (decision
// 19). It writes blocker.closed and restores Working when a holder has
// taken the task up (one not in Assigned), else Submitted. note is optional (<= MaxReason). The notice is
// NoticeUnblocked to the holders other than by. ErrBadState when the task
// isn't Blocked. It resolves every open blocker item as closed, with note
// as the answer (Answer resolves one).
func (c *Coord) Unblock(ctx context.Context, by, id, note string) (Task, Notice, error) {
	return c.impl.unblock(ctx, by, id, note)
}

// AddComment adds a comment to any task, open or done (anyone may; text
// 1..MaxComment runes). wake is the author's per-comment choice (decision
// 18): the notice carries it. The notice is NoticeComment to the holders
// other than by.
func (c *Coord) AddComment(ctx context.Context, by, id, text string, wake bool) (Comment, Notice, error) {
	return c.impl.addComment(ctx, by, id, text, wake, nil)
}

// AddCommentWith is AddComment with files attached (v0.4 §6.6): refs the
// Core made from paths it checked and stored, at most MaxAttachments.
func (c *Coord) AddCommentWith(ctx context.Context, by, id, text string, wake bool, files []blob.FileRef) (Comment, Notice, error) {
	return c.impl.addComment(ctx, by, id, text, wake, files)
}

// CommentAttachment finds a file attached to any Board comment: the ref on
// the earliest comment that carries it, so the answer is stable. Comments
// have no readers, so any viewer may fetch it.
func (c *Coord) CommentAttachment(sha string) (blob.FileRef, bool) {
	return c.impl.commentAttachment(sha)
}

// RecordMessage puts an inbox message about a card on its thread, as a
// comment whose ID is the message id (MsgID, "msg_…"): recording the same
// message again writes nothing, and a Gone record of a message already
// there only marks it gone. Text over MaxComment is cut and points to
// msg:<id>; a Held message is recorded without its text, until a Fill (or
// a Gone record, written only once the policy let it through) brings it;
// Files is only a count. Any task, done ones too (else ErrNotFound). It returns no Notice:
// the recipient already has the message, and the holders read the card.
func (c *Coord) RecordMessage(ctx context.Context, m MessageEntry) (Comment, error) {
	return c.impl.recordMessage(ctx, m)
}

// Comments lists a task's comments, oldest first (nil for no such task).
func (c *Coord) Comments(taskID string) []Comment { return c.impl.listComments(taskID) }

// GetComment returns one comment by id.
func (c *Coord) GetComment(id string) (Comment, bool) { return c.impl.getComment(id) }

// AddLink adds a link to a task (anyone may, open or done; at most
// MaxLinks; the same Ref twice is a no-op). link is parsed with ParseLink.
func (c *Coord) AddLink(ctx context.Context, by, id, link string) (Task, error) {
	return c.impl.addLink(ctx, by, id, link)
}

// RemoveLink drops the link whose Ref equals ref (the creator, a holder or a
// person). ErrNotFound when there is none.
func (c *Coord) RemoveLink(ctx context.Context, by, id, ref string) (Task, error) {
	return c.impl.removeLink(ctx, by, id, ref)
}

// Refusals lists active refusals, newest first.
func (c *Coord) Refusals() []Refusal { return c.impl.activeRefusals() }

// Board groups the tasks for the Board tab and the board tool.
func (c *Coord) Board(f BoardFilter) BoardView { return c.impl.board(f) }

// SweepNotices is Sweep plus blockers that resolved by themselves: a task
// Blocked on a lease that is no longer held, or on a task that is now
// terminal, gets blocker.closed by "irori" (note "lease released" / "task
// completed") and a NoticeUnblocked for its holders. Core calls it in place
// of Sweep and routes the notices from inbox.SystemSender.
func (c *Coord) SweepNotices(ctx context.Context) ([]Notice, error) {
	return c.impl.sweepNotices(ctx)
}

// Search (search.Source): coord owns the task and comment refs. A task's
// Doc has Title = its title, Body = its short ref ("#12"), objective, done criteria, note and
// reason, Agent = Holder, Updated = UpdatedAt; a comment's has Title "comment
// on <task title>", Body = its text, Agent = its author if an agent. Show
// renders a task as its fields, links (label and ref), holders, blocker, and
// its newest 10 comments (Related: "comment:<id>" for older ones); a comment
// as its text with Related ["task:<id>"].

// SearchKinds implements search.Source.
func (c *Coord) SearchKinds() []search.Kind {
	return []search.Kind{search.KindTask, search.KindComment}
}

// SearchDocs implements search.Source.
func (c *Coord) SearchDocs(ctx context.Context, since time.Time, limit int) ([]search.Doc, error) {
	return c.impl.searchDocs(since, limit), nil
}

// SearchShow implements search.Source.
func (c *Coord) SearchShow(ctx context.Context, ref string, o search.ShowOptions) (search.Shown, error) {
	return c.impl.searchShow(ref, o)
}

var _ search.Source = (*Coord)(nil)

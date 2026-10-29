package store

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"sync"
)

// A shared board's log (design/v0.5-shared-boards.md §2): the same schema,
// seqs, Vector, Missing and Insert as the private log, opened with
// OpenOptions.Board. Three rules hold in it that the private log doesn't
// have:
//
//   - the allow-list (§2.2): Append and Insert take only the kinds whose
//     Board column (kindBoards) is BoardAccepts, so a bug can't push
//     capture, presence, inbox or policy rows into a board;
//   - board actors (§2.4): "irori", "person:<personId>" or
//     "agent:<cardId>", never "user", "user:*", "api:*" or an agent's
//     private "<device>/<session>" key;
//   - reference closure (§2.3): Append refuses a row naming a task,
//     comment, blocker, step, card, lease or attachment this log doesn't
//     hold. Insert doesn't check it (a forwarder can't fix another
//     device's rows, and the referenced row may still be on its way); the
//     fold sets such a row aside (Unresolved) until it resolves.
//
// Binding each actor to the writing device's person is the board
// Verifier's (§3, step 2 of §12); the store checks only the shape.

// Errors a board store's Append and Insert report. Match with errors.Is.
var (
	// ErrNotBoardKind: the kind is not on a board log's allow-list.
	ErrNotBoardKind = errors.New("store: kind is not allowed in a board log")
	// ErrBoardActor: the actor is not a board actor (person:, agent:, irori).
	ErrBoardActor = errors.New("store: actor is not allowed in a board log")
	// ErrOutsideBoard: the row references something this board doesn't hold.
	ErrOutsideBoard = errors.New("store: row references something outside this board")
)

// Board is a kind's verdict for a shared board's log (§2.2).
type Board int

const (
	// BoardUnclassified kinds are refused by a board log, like every kind
	// it doesn't name: the allow-list is closed.
	BoardUnclassified Board = iota
	// BoardAccepts: a board log stores it.
	BoardAccepts
	// BoardRefuses: it never enters a board log.
	BoardRefuses
)

// kindBoards is the registry's Board column: for every replicated kind in
// kindSides, whether a board log accepts it (§2.2's two columns).
// TestEveryKindIsClassified fails when a replicated kind is missing here.
var kindBoards = map[string]Board{
	// never: presence, the inbox, capture, requests, the private identity
	KindDeviceStarted: BoardRefuses, KindAgentSeen: BoardRefuses, KindAgentStatus: BoardRefuses, KindAgentGone: BoardRefuses,
	KindPersonDeviceAdded: BoardRefuses, KindPersonDeviceRevoked: BoardRefuses, KindPersonKeyRotated: BoardRefuses,
	KindPersonProfile: BoardRefuses, KindLocalMoved: BoardRefuses,
	"message.sent": BoardRefuses, "delivery.state": BoardRefuses, "wake.issued": BoardRefuses,
	"agent.alias": BoardRefuses, "agent.name": BoardRefuses, "turn.engaged": BoardRefuses, "turn.ended": BoardRefuses,
	"capture.turn": BoardRefuses, "capture.edit": BoardRefuses, "capture.summary": BoardRefuses,
	"capture.commitcmd": BoardRefuses, "pr.ledger": BoardRefuses, "repo.seen": BoardRefuses,
	"commit.seen": BoardRefuses, "commit.linked": BoardRefuses, "decision.recorded": BoardRefuses,
	"delivery.decided": BoardRefuses, "ask.posted": BoardRefuses, "ask.answered": BoardRefuses,
	"ask.resolved": BoardRefuses, "revive.requested": BoardRefuses, KindNodeAddr: BoardRefuses,
	"device.color": BoardRefuses,
	// task.shared names a private task (Share writes a fresh task.created,
	// §5.2). task.edited and the private ask's take rows are not in §2.2's
	// list, so they stay out until the design names them.
	"task.shared": BoardRefuses, "task.edited": BoardRefuses, "take.requested": BoardRefuses, "take.state": BoardRefuses,
	// pinned cards (#35, after the design) are the person's own order in
	// their window, not something the board's other members see
	"task.pinned": BoardRefuses,
	// the Board
	"task.created": BoardAccepts, "task.claimed": BoardAccepts, "task.assigned": BoardAccepts, "task.status": BoardAccepts,
	"task.joined": BoardAccepts, "task.left": BoardAccepts, "task.linked": BoardAccepts, "task.comment": BoardAccepts,
	"task.steps": BoardAccepts, "task.step": BoardAccepts, "task.watched": BoardAccepts,
	// subtasks (#20, after the design): a card's parent is a card on the
	// same board, which closure checks ("parent")
	"task.parent":    BoardAccepts,
	"blocker.closed": BoardAccepts, "blocker.option": BoardAccepts, "blocker.resolved": BoardAccepts,
	// mirrored leases only (§5.4): their writers decide which leases mirror
	"lease.acquired": BoardAccepts, "lease.released": BoardAccepts, "lease.refused": BoardAccepts, "lease.stolen": BoardAccepts,
	KindAttachOffered: BoardAccepts,
	// v0.5: the board's own rows, members' identity objects, cards, review
	KindBoardCreated: BoardAccepts, KindBoardRenamed: BoardAccepts, KindBoardMemberAdded: BoardAccepts,
	KindBoardMemberRole: BoardAccepts, KindBoardMemberRemoved: BoardAccepts, KindBoardMemberLeft: BoardAccepts,
	KindBoardServer: BoardAccepts, KindMemberProfile: BoardAccepts, KindMemberDeviceAdded: BoardAccepts,
	KindMemberDeviceRevoked: BoardAccepts, KindMemberKeyRotated: BoardAccepts, KindAgentCard: BoardAccepts,
	KindReviewRequested: BoardAccepts, KindReviewReviewer: BoardAccepts,
	// v0.5 private-log rows
	KindBoardJoined: BoardRefuses, KindBoardForgot: BoardRefuses, KindReceiveSet: BoardRefuses, KindAutonomySet: BoardRefuses,
	KindSharedCopy: BoardRefuses, KindBoardCardOf: BoardRefuses,
}

// KindBoard reports kind's Board column.
func KindBoard(kind string) Board { return kindBoards[kind] }

// BoardAccepted reports whether a board log stores kind: only the kinds the
// registry marks BoardAccepts, never a device-local one.
func BoardAccepted(kind string) bool { return !IsLocal(kind) && kindBoards[kind] == BoardAccepts }

// IsBoardIdentityKind reports a board log's identity row (board.*,
// member.*, and agent.card, which binds a card to its person), which a
// board store's Verifier folds as a private store's folds person.* (§3,
// §2.4).
func IsBoardIdentityKind(kind string) bool {
	return BoardAccepted(kind) && (strings.HasPrefix(kind, "board.") || strings.HasPrefix(kind, "member.") || kind == KindAgentCard)
}

// Board actor prefixes (§2.4).
const (
	PersonActorPrefix = "person:"
	AgentActorPrefix  = "agent:"
)

// BoardActorOK reports whether actor may write a board row: "irori",
// "person:<personId>" or "agent:<cardId>", where the id has no ':', '/'
// or space (an agent's private key, "<device>/<session>", names its device
// and session, which never go on a board).
func BoardActorOK(actor string) bool {
	if actor == "irori" {
		return true
	}
	id, ok := strings.CutPrefix(actor, PersonActorPrefix)
	if !ok {
		id, ok = strings.CutPrefix(actor, AgentActorPrefix)
	}
	return ok && boardID(id)
}

// boardID is a person or card id as a board key carries it.
func boardID(id string) bool {
	return id != "" && len(id) <= 128 && !strings.ContainsAny(id, ":/\\ \t\r\n")
}

// BoardCounts are the rows a board store refused (§2.2: "refused and
// counted"), since it opened.
type BoardCounts struct {
	Kind  int `json:"kind"`  // not on the allow-list
	Actor int `json:"actor"` // not a board actor
	// Outside: Append calls refused for a reference outside the board.
	Outside int `json:"outside"`
}

// IsBoard reports a store opened with OpenOptions.Board.
func (s *Store) IsBoard() bool { return s.impl.board != nil }

// BoardRefused reports what a board store refused; zero for the private log.
func (s *Store) BoardRefused() BoardCounts {
	b := s.impl.board
	if b == nil {
		return BoardCounts{}
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.counts
}

// Unresolved reports why ev's references don't all resolve in this board
// log right now (an error wrapping ErrOutsideBoard), or nil. The private log
// always resolves. The fold calls it to set a forwarded row aside until
// what it names arrives (§2.3).
func (s *Store) Unresolved(ev Event) error {
	b := s.impl.board
	if b == nil {
		return nil
	}
	return b.resolves(ev.Kind, ev.Payload)
}

// boardRules is a board store's state: what it holds that rows may
// reference, and what it refused.
type boardRules struct {
	mu       sync.Mutex
	tasks    map[string]bool
	comments map[string]bool
	blockers map[string]bool // taskId "/" blockerId
	steps    map[string]bool // taskId "/" stepId
	cards    map[string]bool
	hashes   map[string]bool
	leases   map[string]bool // resource
	persons  map[string]bool // personId: board.created's creator, board.member.added's person
	counts   BoardCounts
}

func newBoardRules() *boardRules {
	return &boardRules{tasks: map[string]bool{}, comments: map[string]bool{}, blockers: map[string]bool{},
		steps: map[string]bool{}, cards: map[string]bool{}, hashes: map[string]bool{}, leases: map[string]bool{},
		persons: map[string]bool{}}
}

// boardRow is every field of a board row that defines or names an id.
type boardRow struct {
	TaskID    string     `json:"taskId"`
	Parent    string     `json:"parent"`
	CommentID string     `json:"commentId"`
	BlockerID string     `json:"blockerId"`
	StepID    string     `json:"stepId"`
	MsgID     string     `json:"msgId"`
	CardID    string     `json:"cardId"`
	SHA256    string     `json:"sha256"`
	Resource  string     `json:"resource"`
	Held      string     `json:"held"`
	By        string     `json:"by"`
	To        string     `json:"to"`
	Who       string     `json:"who"`
	Holder    string     `json:"holder"`
	HeldBy    string     `json:"heldBy"`
	BlockedBy *boardOn   `json:"blockedBy"`
	Blocker   *boardItem `json:"blocker"`
	Files     []boardRef `json:"attachments"`
	Steps     []struct {
		ID string `json:"id"`
	} `json:"steps"`
	Stolen []struct {
		Resource string `json:"resource"`
		Holder   string `json:"holder"`
	} `json:"stolen"`
	// StolenFrom and Conflict: holder keys a lease row names (a comma
	// list, and the other holder of a partition conflict).
	StolenFrom string `json:"stolenFrom"`
	Conflict   string `json:"conflict"`
	// Outcome: an ask's outcome names the card it made or changed.
	Outcome *struct {
		Task string `json:"task"`
	} `json:"outcome"`
	PersonID string `json:"personId"`
	Creator  *struct {
		PersonID string `json:"personId"`
	} `json:"creator"`
}

type boardOn struct {
	Kind string `json:"kind"`
	Ref  string `json:"ref"`
}

type boardItem struct {
	ID    string     `json:"id"`
	On    *boardOn   `json:"on"`
	Files []boardRef `json:"attachments"`
}

type boardRef struct {
	SHA256 string `json:"sha256"`
}

// checkShape is what a board store checks on every row, appended or
// inserted: the allow-list and the actor. The caller holds the store's
// write lock.
func (b *boardRules) checkShape(actor, kind string, payload []byte) error {
	if !BoardAccepted(kind) {
		b.count(func(c *BoardCounts) { c.Kind++ })
		return fmt.Errorf("%w: %s", ErrNotBoardKind, kind)
	}
	if !BoardActorOK(actor) {
		b.count(func(c *BoardCounts) { c.Actor++ })
		return fmt.Errorf("%w: %q", ErrBoardActor, actor)
	}
	if kind == KindAgentCard {
		// A card is written by the agent it describes, and nobody else.
		var p boardRow
		if json.Unmarshal(payload, &p) != nil || actor != AgentActorPrefix+p.CardID {
			b.count(func(c *BoardCounts) { c.Actor++ })
			return fmt.Errorf("%w: agent.card for %q written as %q", ErrBoardActor, p.CardID, actor)
		}
	}
	return nil
}

func (b *boardRules) count(f func(*BoardCounts)) {
	b.mu.Lock()
	f(&b.counts)
	b.mu.Unlock()
}

// define records the ids a stored row brings into the board.
func (b *boardRules) define(kind string, payload []byte) {
	var p boardRow
	if json.Unmarshal(payload, &p) != nil {
		return
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	switch kind {
	case "task.created":
		if p.TaskID != "" {
			b.tasks[p.TaskID] = true
		}
	case "task.comment":
		if p.CommentID != "" {
			b.comments[p.CommentID] = true
		}
	case "task.status":
		if p.Blocker != nil && p.Blocker.ID != "" {
			b.blockers[p.TaskID+"/"+p.Blocker.ID] = true
		}
	case "task.steps":
		for _, s := range p.Steps {
			b.steps[p.TaskID+"/"+s.ID] = true
		}
	case KindAgentCard:
		b.cards[p.CardID] = true
	case KindAttachOffered:
		if p.SHA256 != "" {
			b.hashes[p.SHA256] = true
		}
	case "lease.acquired":
		if p.Resource != "" {
			b.leases[p.Resource] = true
		}
	case KindBoardCreated:
		if p.Creator != nil && boardID(p.Creator.PersonID) {
			b.persons[p.Creator.PersonID] = true
		}
	case KindBoardMemberAdded:
		if boardID(p.PersonID) {
			b.persons[p.PersonID] = true
		}
	}
}

// resolves is the closure check (§2.3): nil when every id kind's row names
// is held in this board, else the first one that isn't, by field. Commit
// SHAs, PR URLs and other links are not ids and always pass. A person:
// key resolves when board.created or a board.member.added in this log
// names the person: whether that membership counts is the Verifier's
// (closure is about meaning, not authority).
func (b *boardRules) resolves(kind string, payload []byte) error {
	if strings.HasPrefix(kind, "member.") || strings.HasPrefix(kind, "board.") {
		// A member's identity objects and the board's own rows name only
		// persons (as bare personIds) and devices (member.key.rotated's by
		// is a device id), which the board Verifier checks; nothing in
		// them is a board reference.
		return nil
	}
	var p boardRow
	if err := json.Unmarshal(payload, &p); err != nil {
		return fmt.Errorf("%w: %s payload is not an object", ErrOutsideBoard, kind)
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	out := func(format string, a ...any) error {
		return fmt.Errorf("%w: %s: %s", ErrOutsideBoard, kind, fmt.Sprintf(format, a...))
	}
	if p.TaskID != "" && kind != "task.created" && !b.tasks[p.TaskID] {
		return out("taskId %s is not a card on this board", p.TaskID)
	}
	if p.Parent != "" && !b.tasks[p.Parent] {
		return out("parent %s is not a card on this board", p.Parent)
	}
	if p.BlockerID != "" && !b.blockers[p.TaskID+"/"+p.BlockerID] {
		return out("blockerId %s is not a blocker of that card on this board", p.BlockerID)
	}
	if p.StepID != "" && !b.steps[p.TaskID+"/"+p.StepID] {
		return out("stepId %s is not a step of that card on this board", p.StepID)
	}
	if p.CommentID != "" && kind != "task.comment" && !b.comments[p.CommentID] {
		return out("commentId %s is not a comment on this board", p.CommentID)
	}
	if p.MsgID != "" {
		return out("msgId names a private message; messages never go on a board")
	}
	self := ""
	if kind == KindAgentCard {
		self = p.CardID
	}
	keys := []struct{ field, key string }{{"by", p.By}, {"to", p.To}, {"who", p.Who}, {"holder", p.Holder}, {"heldBy", p.HeldBy}}
	for _, s := range p.Stolen {
		keys = append(keys, struct{ field, key string }{"stolen.holder", s.Holder})
		if !b.leases[s.Resource] {
			return out("stolen lease %s is not on this board", s.Resource)
		}
	}
	for _, h := range strings.Split(p.StolenFrom, ",") {
		keys = append(keys, struct{ field, key string }{"stolenFrom", strings.TrimSpace(h)})
	}
	keys = append(keys, struct{ field, key string }{"conflict", p.Conflict})
	if p.Outcome != nil && p.Outcome.Task != "" && !b.tasks[p.Outcome.Task] {
		return out("its outcome names task %s, which is not on this board", p.Outcome.Task)
	}
	for _, k := range keys {
		if err := b.key(k.key, self); err != "" {
			return out("%s %q %s", k.field, k.key, err)
		}
	}
	files := p.Files
	ons := []*boardOn{p.BlockedBy}
	if p.Blocker != nil {
		files = append(files, p.Blocker.Files...)
		ons = append(ons, p.Blocker.On)
	}
	for _, f := range files {
		if !b.hashes[f.SHA256] {
			return out("attachment %s was not offered on this board", f.SHA256)
		}
	}
	for _, on := range ons {
		if on == nil {
			continue
		}
		switch on.Kind {
		case "task":
			if !b.tasks[on.Ref] {
				return out("it waits on task %s, which is not on this board", on.Ref)
			}
		case "lease":
			if !b.leases[on.Ref] {
				return out("it waits on lease %s, which is not on this board", on.Ref)
			}
		case "agent", "person":
			if err := b.key(on.Ref, self); err != "" {
				return out("it waits on %q, which %s", on.Ref, err)
			}
		}
	}
	switch kind {
	case "lease.released", "lease.stolen":
		if !b.leases[p.Resource] {
			return out("lease %s is not on this board", p.Resource)
		}
	case "lease.refused":
		if p.Held != "" && !b.leases[p.Held] {
			return out("lease %s is not on this board", p.Held)
		}
	}
	return nil
}

// key checks one actor-like key a row names; "" when it is fine, else why
// not. b.mu is held.
func (b *boardRules) key(k, selfCard string) string {
	switch {
	case k == "" || k == "irori":
		return ""
	case strings.HasPrefix(k, PersonActorPrefix):
		id := k[len(PersonActorPrefix):]
		if !boardID(id) {
			return "is not a person's board key"
		}
		if !b.persons[id] {
			return "is not a person on this board"
		}
		return ""
	case strings.HasPrefix(k, AgentActorPrefix):
		id := k[len(AgentActorPrefix):]
		if !boardID(id) || (id != selfCard && !b.cards[id]) {
			return "is not an agent card on this board"
		}
		return ""
	}
	return "is a private key; a board names only person: and agent: keys"
}

// loadBoard builds a board store's index from every row it holds.
func (d *db) loadBoard(ctx context.Context) error {
	var after Cursor
	for {
		evs, err := (&Store{impl: d}).list(ctx, Filter{After: after, Limit: MaxListLimit})
		if err != nil {
			return err
		}
		for _, ev := range evs {
			d.board.define(ev.Kind, ev.Payload)
		}
		if len(evs) < MaxListLimit {
			return nil
		}
		after = evs[len(evs)-1].Cursor()
	}
}

// identityKind reports the rows this store's Verifier folds: person.* in
// the private log, board.*, member.* and agent.card in a board log.
func (d *db) identityKind(kind string) bool {
	if d.board != nil {
		return IsBoardIdentityKind(kind)
	}
	return IsPersonKind(kind)
}

// identityPrefixes are identityKind's rows as List prefixes.
func (d *db) identityPrefixes() []string {
	if d.board != nil {
		return []string{"board.", "member.", KindAgentCard}
	}
	return []string{"person."}
}

// revokeKinds are the kinds honoured past their writer's cut: the revoke,
// and in a board log the removal and the leave too, since each makes cuts
// and cuts must not depend on which row arrived first (two owners who
// remove each other while apart are both removed, as a mutual revoke).
func (d *db) revokeKinds() []string {
	if d.board != nil {
		return []string{KindMemberDeviceRevoked, KindBoardMemberRemoved, KindBoardMemberLeft}
	}
	return []string{KindPersonDeviceRevoked}
}

// revokeKind reports one of revokeKinds.
func (d *db) revokeKind(kind string) bool {
	for _, k := range d.revokeKinds() {
		if k == kind {
			return true
		}
	}
	return false
}

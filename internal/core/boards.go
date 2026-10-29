package core

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/owner"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// Shared boards, step 1 (design/v0.5-shared-boards.md §2, §12): each board
// is its own log, boards/<boardId>.db, opened by the store package in board
// mode with this device's Signer and a board Verifier, and folded by its
// own coord. The Core holds them all and routes a taskId to the one log it
// lives in (Route); tools and the window go through that.
//
// What is not here yet, by the design's build order: board.created and
// membership (step 3), the board Verifier's per-member chains (step 2),
// connections and sync (step 5), tags in short refs and the Boards dialog
// (step 4). Until then a board holds this person's rows only and stays on
// this device.

// BoardsDir is where the board logs live, under the data dir.
const BoardsDir = "boards"

// ErrNoBoard: no board log with that id on this device.
var ErrNoBoard = errors.New("core: no such board on this device")

// boardLog is one open board.
type boardLog struct {
	id    string
	store *store.Store
	coord *coord.Coord

	mu    sync.Mutex
	cards map[string]boardCard // cardId -> its agent.card, as folded
	keys  map[string]string    // agentKey -> cardId: this device's board_cards
	byID  map[string]string    // cardId -> agentKey
}

// boardCard is a folded agent.card row.
type boardCard struct {
	CardID   string `json:"cardId"`
	Name     string `json:"name"`
	Kind     string `json:"kind"`
	PersonID string `json:"personId"`
	Status   string `json:"status"`
}

// cardKeyRow is the device-local board_cards row (store.KindBoardCardKey).
type cardKeyRow struct {
	CardID   string `json:"cardId"`
	AgentKey string `json:"agentKey"`
}

// boardState is the Core's set of board logs and the taskId index.
type boardState struct {
	mu     sync.Mutex
	logs   map[string]*boardLog
	taskOf map[string]string // taskId -> boardId, from each log's task.created rows
	ctx    context.Context   // the Core's; each board's goroutines end with it
	wg     sync.WaitGroup
}

// NewBoardID is a fresh boardId: 16 random bytes, hex (§2.1). It names
// nothing.
func NewBoardID() (string, error) {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", err
	}
	return hex.EncodeToString(b[:]), nil
}

// validBoardID reports a boardId as NewBoardID makes them, so a file name
// can't leave the boards dir.
func validBoardID(id string) bool {
	if len(id) != 32 {
		return false
	}
	_, err := hex.DecodeString(id)
	return err == nil && strings.ToLower(id) == id
}

// openBoards opens every board log in the data dir. A board that fails to
// open is logged and left out: the private board keeps working.
func (c *Core) openBoards(ctx context.Context) {
	c.boards.ctx = ctx
	ents, err := os.ReadDir(filepath.Join(c.opts.DataDir, BoardsDir))
	if err != nil {
		if !errors.Is(err, os.ErrNotExist) {
			log.Printf("boards: %v", err)
		}
		return
	}
	for _, e := range ents {
		id, ok := strings.CutSuffix(e.Name(), ".db")
		if e.IsDir() || !ok || !validBoardID(id) {
			continue
		}
		if _, err := c.openBoard(ctx, id); err != nil {
			log.Printf("board %s: %v", id, err)
		}
	}
}

// CreateBoard makes a new, empty board log on this device and returns its
// id. Step 3 of §12 writes its board.created row; step 4 puts it behind
// the Boards dialog.
func (c *Core) CreateBoard(ctx context.Context) (string, error) {
	id, err := NewBoardID()
	if err != nil {
		return "", err
	}
	if _, err := c.openBoard(ctx, id); err != nil {
		return "", err
	}
	return id, nil
}

// openBoard opens (creating) one board log and its coord, and starts
// following it.
func (c *Core) openBoard(ctx context.Context, id string) (*boardLog, error) {
	if !validBoardID(id) {
		return nil, fmt.Errorf("core: %q is not a board id", id)
	}
	c.boards.mu.Lock()
	defer c.boards.mu.Unlock()
	if b := c.boards.logs[id]; b != nil {
		return b, nil
	}
	st, err := store.OpenWith(ctx, filepath.Join(c.opts.DataDir, BoardsDir, id+".db"), c.device.ID, hlc.New(c.opts.Now),
		store.OpenOptions{Signer: c.ident, Verifier: ownBoardVerifier{c.chain}, Now: c.opts.Now, Board: true})
	if err != nil {
		return nil, fmt.Errorf("open board log: %w", err)
	}
	b := &boardLog{id: id, store: st, cards: map[string]boardCard{}, keys: map[string]string{}, byID: map[string]string{}}
	if err := b.load(ctx); err != nil {
		st.Close()
		return nil, err
	}
	me := store.PersonActorPrefix + c.chain.PersonID()
	if b.coord, err = coord.New(ctx, st, boardRoster{c, b}, coord.Options{Now: c.opts.Now, Board: true, Person: me,
		IsOwner: func(k string) bool { return k == store.PersonActorPrefix+c.chain.PersonID() }}); err != nil {
		st.Close()
		return nil, fmt.Errorf("board coord: %w", err)
	}
	if c.boards.logs == nil {
		c.boards.logs, c.boards.taskOf = map[string]*boardLog{}, map[string]string{}
	}
	tasks, err := b.taskIDs(ctx)
	if err != nil {
		st.Close()
		return nil, err
	}
	for _, t := range tasks {
		c.boards.taskOf[t] = id
	}
	c.boards.logs[id] = b
	fctx := c.boards.ctx
	if fctx == nil {
		fctx = ctx
	}
	c.boards.wg.Add(2)
	go func() { defer c.boards.wg.Done(); b.coord.Follow(fctx) }()
	go func() { defer c.boards.wg.Done(); c.followBoard(fctx, b) }()
	return b, nil
}

// closeBoards waits for the boards' goroutines (the Core's context is
// already canceled) and closes their logs.
func (c *Core) closeBoards() {
	done := make(chan struct{})
	go func() { c.boards.wg.Wait(); close(done) }()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
	}
	c.boards.mu.Lock()
	defer c.boards.mu.Unlock()
	for id, b := range c.boards.logs {
		b.store.Close()
		delete(c.boards.logs, id)
	}
}

// load folds the log's agent cards and this device's board_cards.
func (b *boardLog) load(ctx context.Context) error {
	var after store.Cursor
	for {
		evs, err := b.store.List(ctx, store.Filter{Kinds: []string{store.KindAgentCard}, After: after, Limit: store.MaxListLimit})
		if err != nil {
			return err
		}
		for _, ev := range evs {
			b.foldCard(ev)
		}
		if len(evs) < store.MaxListLimit {
			break
		}
		after = evs[len(evs)-1].Cursor()
	}
	keys, err := b.store.List(ctx, store.Filter{Kinds: []string{store.KindBoardCardKey}, Local: true, Limit: store.MaxListLimit})
	if err != nil {
		return err
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	for _, ev := range keys {
		var r cardKeyRow
		if json.Unmarshal(ev.Payload, &r) == nil && r.CardID != "" && r.AgentKey != "" {
			b.keys[r.AgentKey], b.byID[r.CardID] = r.CardID, r.AgentKey
		}
	}
	return nil
}

func (b *boardLog) foldCard(ev store.Event) {
	var p boardCard
	if json.Unmarshal(ev.Payload, &p) != nil || p.CardID == "" {
		return
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	b.cards[p.CardID] = p
}

// taskIDs lists every task.created row's taskId in the log.
func (b *boardLog) taskIDs(ctx context.Context) ([]string, error) {
	var out []string
	var after store.Cursor
	for {
		evs, err := b.store.List(ctx, store.Filter{Kinds: []string{coord.KindTaskCreated}, After: after, Limit: store.MaxListLimit})
		if err != nil {
			return nil, err
		}
		for _, ev := range evs {
			if id := createdTaskID(ev); id != "" {
				out = append(out, id)
			}
		}
		if len(evs) < store.MaxListLimit {
			return out, nil
		}
		after = evs[len(evs)-1].Cursor()
	}
}

func createdTaskID(ev store.Event) string {
	var p struct {
		TaskID string `json:"taskId"`
	}
	_ = json.Unmarshal(ev.Payload, &p)
	return p.TaskID
}

// followBoard keeps the taskId index and the cards current as rows reach
// b's log, rebuilding them on a resync.
func (c *Core) followBoard(ctx context.Context, b *boardLog) {
	for ev := range b.store.Subscribe(ctx) {
		switch {
		case ev.Seq == 0:
			ids, err := b.taskIDs(ctx)
			if err != nil {
				continue
			}
			c.boards.mu.Lock()
			for _, id := range ids {
				c.boards.taskOf[id] = b.id
			}
			c.boards.mu.Unlock()
			_ = b.load(ctx)
		case ev.Kind == coord.KindTaskCreated:
			if id := createdTaskID(ev); id != "" {
				c.boards.mu.Lock()
				c.boards.taskOf[id] = b.id
				c.boards.mu.Unlock()
			}
		case ev.Kind == store.KindAgentCard:
			b.foldCard(ev)
		}
	}
}

// Boards lists the ids of the board logs open on this device, sorted.
func (c *Core) Boards() []string {
	c.boards.mu.Lock()
	defer c.boards.mu.Unlock()
	out := make([]string, 0, len(c.boards.logs))
	for id := range c.boards.logs {
		out = append(out, id)
	}
	sort.Strings(out)
	return out
}

// BoardCoord is one board's coord, or ErrNoBoard.
func (c *Core) BoardCoord(boardID string) (*coord.Coord, error) {
	b, err := c.board(boardID)
	if err != nil {
		return nil, err
	}
	return b.coord, nil
}

func (c *Core) board(boardID string) (*boardLog, error) {
	c.boards.mu.Lock()
	defer c.boards.mu.Unlock()
	if b := c.boards.logs[boardID]; b != nil {
		return b, nil
	}
	return nil, fmt.Errorf("%w: %s", ErrNoBoard, boardID)
}

// Route is the one routing function from a taskId to its log (§2.1): ""
// and the private coord for a card on the private board, else the board
// whose log holds its task.created row. A task lives in exactly one log;
// an id in none is coord.ErrNotFound.
func (c *Core) Route(taskID string) (boardID string, co *coord.Coord, err error) {
	if _, ok := c.coord.GetTask(taskID); ok {
		return "", c.coord, nil
	}
	c.boards.mu.Lock()
	defer c.boards.mu.Unlock()
	if id, ok := c.boards.taskOf[taskID]; ok && c.boards.logs[id] != nil {
		return id, c.boards.logs[id].coord, nil
	}
	// A row this device just appended reaches followBoard a moment later;
	// its coord already has it.
	for id, b := range c.boards.logs {
		if _, ok := b.coord.GetTask(taskID); ok {
			c.boards.taskOf[taskID] = id
			return id, b.coord, nil
		}
	}
	return "", nil, fmt.Errorf("%w: task %q", coord.ErrNotFound, taskID)
}

// TaskCoord routes taskID (Route) and gives each of keys as the caller is
// known in that log: unchanged on the private board, a board key on a
// shared one (BoardKey). An id in no log goes to the private coord, which
// reports it as not found in its own words. Tools and the window act on a
// card through it (tools.TaskRouter).
func (c *Core) TaskCoord(ctx context.Context, taskID string, keys ...string) (*coord.Coord, []string, error) {
	boardID, co, err := c.Route(taskID)
	if err != nil || boardID == "" {
		return c.coord, keys, nil
	}
	out := make([]string, len(keys))
	for i, k := range keys {
		if out[i], err = c.BoardKey(ctx, boardID, k); err != nil {
			return nil, nil, err
		}
	}
	return co, out, nil
}

// BoardKey is key as it is written on board boardID (§2.4): the person at
// this device ("user:<device>" that IsOwner accepts) becomes
// person:<personId>, written only through the GUI since nothing else
// holds a user: key; an agent on this device becomes agent:<cardId>, its
// card written the first time it touches the board; "irori" and keys
// already in board form are kept. Anything else is refused: another
// device's agent gets its card from its own device, and a made-up user:
// key is no one.
func (c *Core) BoardKey(ctx context.Context, boardID, key string) (string, error) {
	key = strings.TrimSpace(key)
	switch {
	case key == "" || key == "irori":
		return key, nil
	case strings.HasPrefix(key, store.PersonActorPrefix) || strings.HasPrefix(key, store.AgentActorPrefix):
		return key, nil
	case c.chain.IsOwner(key):
		return store.PersonActorPrefix + c.chain.PersonID(), nil
	case strings.HasPrefix(key, identity.UserKeyPrefix):
		return "", fmt.Errorf("%w: %q is not a person on this board", coord.ErrNotAllowed, key)
	}
	agent := strings.TrimPrefix(key, "api:")
	if owner.DeviceOf(agent) != c.device.ID {
		return "", fmt.Errorf("%w: %q is not an agent on this device; its own device puts it on a board", coord.ErrNotAllowed, agent)
	}
	b, err := c.board(boardID)
	if err != nil {
		return "", err
	}
	return c.cardFor(ctx, b, agent)
}

// cardFor is agent's cardId key on b, minting the card on first use: a
// random cardId per (agent, board), so two boards can't be joined on it,
// an agent.card with nothing but its name, kind and person, and the
// device-local board_cards row that says which session it is (§2.4).
func (c *Core) cardFor(ctx context.Context, b *boardLog, agent string) (string, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if id := b.keys[agent]; id != "" {
		return store.AgentActorPrefix + id, nil
	}
	var card *presence.AgentCard
	for _, a := range c.LocalAgents() {
		if a.Key == agent {
			card = &a
			break
		}
	}
	if card == nil {
		return "", fmt.Errorf("%w: no agent %q on this device", coord.ErrNotFound, agent)
	}
	var raw [16]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return "", err
	}
	id := hex.EncodeToString(raw[:])
	kind := card.Agent
	if kind == "" {
		kind = "claude"
	}
	row := boardCard{CardID: id, Name: card.Name, Kind: kind, PersonID: c.chain.PersonID(), Status: "active"}
	// The local row first: a card whose session nobody knows is worse
	// than a mapping to a card that failed to append (it is retried).
	if _, err := b.store.AppendLocal(ctx, "irori", store.KindBoardCardKey, cardKeyRow{CardID: id, AgentKey: agent}); err != nil {
		return "", err
	}
	if _, err := b.store.Append(ctx, store.AgentActorPrefix+id, store.KindAgentCard, row); err != nil {
		return "", err
	}
	b.keys[agent], b.byID[id], b.cards[id] = id, agent, row
	return store.AgentActorPrefix + id, nil
}

// PrivateKey is a board key as this device knows it: the session key of
// one of this device's cards (board_cards), the person at this device for
// its own person: key, or key unchanged.
func (c *Core) PrivateKey(key string) string {
	if key == store.PersonActorPrefix+c.chain.PersonID() {
		return c.UserSender().Key
	}
	id, ok := strings.CutPrefix(key, store.AgentActorPrefix)
	if !ok {
		return key
	}
	c.boards.mu.Lock()
	logs := make([]*boardLog, 0, len(c.boards.logs))
	for _, b := range c.boards.logs {
		logs = append(logs, b)
	}
	c.boards.mu.Unlock()
	for _, b := range logs {
		b.mu.Lock()
		agent := b.byID[id]
		b.mu.Unlock()
		if agent != "" {
			return agent
		}
	}
	return key
}

// boardRoster is the roster a board's coord reads liveness from: each of
// the log's agent cards, keyed agent:<cardId>. One of this device's cards
// is as live as its session (missing when the session is); another's
// shows the status its card row says.
type boardRoster struct {
	c *Core
	b *boardLog
}

func (r boardRoster) Agents() []presence.AgentCard {
	live := map[string]presence.AgentCard{}
	for _, a := range r.c.MeshAgents() {
		live[a.Key] = a
	}
	r.b.mu.Lock()
	defer r.b.mu.Unlock()
	out := make([]presence.AgentCard, 0, len(r.b.cards))
	for id, card := range r.b.cards {
		a := presence.AgentCard{Key: store.AgentActorPrefix + id, Name: card.Name, Agent: card.Kind, Status: card.Status}
		if agent := r.b.byID[id]; agent != "" {
			s, ok := live[agent]
			if !ok {
				continue
			}
			a.Status, a.UpdatedAt = s.Status, s.UpdatedAt
		}
		out = append(out, a)
	}
	return out
}

// ownBoardVerifier is the board Verifier until step 2's per-member chains
// (§3): a board row must be signed by one of this person's devices that
// is certified and not revoked. Board cuts are board-log seqs, which the
// private chain doesn't have, so a revoked device's board rows are refused
// whole rather than above a cut.
type ownBoardVerifier struct{ chain *identity.Chain }

func (v ownBoardVerifier) VerifyRow(ev store.Event) error {
	if _, revoked := v.chain.Cut(ev.DeviceID); revoked {
		return fmt.Errorf("%w: %s was removed", store.ErrPastCut, ev.DeviceID)
	}
	if _, ok := v.chain.Cert(ev.DeviceID); !ok {
		return fmt.Errorf("%w: %s", store.ErrNoCert, ev.DeviceID)
	}
	return v.chain.VerifyRow(ev)
}
func (ownBoardVerifier) Cut(string) (int64, bool)    { return 0, false }
func (ownBoardVerifier) CutClaims() []store.CutClaim { return nil }
func (ownBoardVerifier) Observe(store.Event)         {}
func (ownBoardVerifier) Reset()                      {}

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
// Each board's Verifier is identity.BoardVerifier (§3, step 2): one chain
// per pinned member, seeded from the member's pin and grown by the
// member.* rows each member's devices write (keepCurrent writes this
// person's). The device-local board.pin row is the board's anchor: its
// creator as this device trusts it, which board.created must match (from
// its own creation; an invite bundle's in stage 2). Every other member is
// pinned by the membership fold from owners' board.member.added rows
// (step 3), which also cuts removed and departed members.
//
// Each board's secret (§6.1) sits beside its log, boards/<id>.secrets,
// sealed by the keystore and never in a row. rotateSecret mints the next
// epoch's when a removal, a leave or a member device revoke folds.
//
// What is not here yet, by the design's build order: connections and
// sync (step 5), tags in short refs and the Boards dialog (step 4), and the
// invite, remove and leave write paths (§4.1, §4.3, §4.4, with their
// cleanup and forgetting). Until then a board holds this person's rows
// only and stays on this device.

// BoardsDir is where the board logs live, under the data dir.
const BoardsDir = "boards"

// ErrNoBoard: no board log with that id on this device.
var ErrNoBoard = errors.New("core: no such board on this device")

// boardLog is one open board.
type boardLog struct {
	id       string
	store    *store.Store
	coord    *coord.Coord
	verifier *identity.BoardVerifier
	secrets  *identity.BoardSecrets

	idMu sync.Mutex // one keepCurrent or rotateSecret at a time

	// cancel ends the board's goroutines (wg), so one board can be
	// forgotten while the Core runs (boards_own.go).
	cancel context.CancelFunc
	wg     sync.WaitGroup

	mu    sync.Mutex
	cards map[string]boardCard // cardId -> its agent.card, as folded
	keys  map[string]string    // agentKey -> cardId: the person's board_cards (addKey)
	byID  map[string]string    // cardId -> agentKey
}

// addKey records that board card cardID is the session agent. b.mu is
// held. Two of the person's devices can mint a card for the same agent
// while apart: both cards map back to it, and the lower cardId is the one
// written from here on, so every device settles on the same card.
func (b *boardLog) addKey(cardID, agent string) {
	b.byID[cardID] = agent
	if cur := b.keys[agent]; cur == "" || cardID < cur {
		b.keys[agent] = cardID
	}
}

// boardCard is a folded agent.card row.
type boardCard struct {
	CardID   string `json:"cardId"`
	Name     string `json:"name"`
	Kind     string `json:"kind"`
	PersonID string `json:"personId"`
	Status   string `json:"status"`
}

// cardKeyRow is a board_cards row: which session a board's agent card is
// (§2.4). It is a private row replicated to the person's own devices
// (store.KindBoardCardOf, with boardId), so every one of them translates a
// card both ways: an agent on the laptop is assigned from the PC, and a
// notice about its card reaches its inbox from any device. It names a
// session key and a boardId, both private already, and never goes on a
// board. Rows from before it are device-local in the board store
// (store.KindBoardCardKey, no boardId) and are still read.
type cardKeyRow struct {
	BoardID  string `json:"boardId,omitempty"`
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
	follow sync.Once // followIdentity

	// Step 4 (boards_own.go): one reconcile of board.joined and
	// board.forgot at a time, this device's shared copies (§5.2) by private
	// task id, and the subscribers to every board's changes.
	recMu  sync.Mutex
	copies map[string]sharedCopy
	subMu  sync.Mutex
	subs   map[chan coord.Change]struct{}
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
		if _, err := c.openBoard(ctx, id, false, nil); err != nil {
			log.Printf("board %s: %v", id, err)
		}
	}
}

// CreateBoard makes a new board log on this device and returns its id
// (§2.1). It anchors this person with this device as the board root,
// writes this person's member.* rows (§3) and then board.created, signed by
// the person key: the self-cert comes first in this device's stream, so a
// peer can check board.created's device signature against it. The board's
// first secret is made with it (§6.1). name is shown on the board; tag is
// its short handle in refs (`mlg#12`, B3), unique among the boards on this
// device. Step 4 puts it behind the Boards dialog.
func (c *Core) CreateBoard(ctx context.Context, name, tag string) (string, error) {
	name = strings.TrimSpace(name)
	if !identity.ValidBoardName(name) {
		return "", fmt.Errorf("%w: a board name is 1 to %d characters", coord.ErrInvalid, identity.MaxBoardName)
	}
	if !identity.ValidBoardTag(tag) {
		return "", fmt.Errorf("%w: a board tag is a lowercase letter, then up to 15 lowercase letters, digits or '-'", coord.ErrInvalid)
	}
	if other := c.tagHolder(tag); other != "" {
		return "", fmt.Errorf("%w: the tag %q is already board %s's", coord.ErrInvalid, tag, other)
	}
	id, err := NewBoardID()
	if err != nil {
		return "", err
	}
	b, err := c.openBoard(ctx, id, true, nil)
	if err != nil {
		return "", err
	}
	// Until board.joined is written, a failure takes the board's files
	// with it: a half-made board would open again at the next start,
	// nameless and never ready.
	fail := func(err error) (string, error) {
		if rerr := c.removeBoard(id); rerr != nil {
			log.Printf("board %s: clean up a failed creation: %v", id, rerr)
		}
		return "", err
	}
	pin, _ := b.verifier.Pinned(c.chain.PersonID())
	row, err := c.ident.BoardCreatedRow(id, name, tag, pin, c.now())
	if err != nil {
		return fail(err)
	}
	if _, err := b.store.Append(ctx, store.PersonActorPrefix+pin.PersonID, store.KindBoardCreated, row); err != nil {
		return fail(err)
	}
	if !b.verifier.Membership().Created {
		return fail(errors.New("core: the board's board.created did not count"))
	}
	// The person's other devices open it from this row (§4.1 step 7, own
	// devices only until invites): it carries the creator's pin, which they
	// trust because one of their own devices wrote it.
	if _, err := c.store.Append(ctx, "irori", store.KindBoardJoined, boardJoinedRow{BoardID: id, Name: name, Tag: tag, Creator: pin, At: c.now()}); err != nil {
		return fail(err)
	}
	// The board is made; a secret that fails here is made again when the
	// board next opens or a row moves its epoch (followBoard).
	if err := c.rotateSecret(b); err != nil {
		log.Printf("board %s: secret: %v", id, err)
	}
	c.boardsMoved()
	return id, nil
}

// tagHolder is the board that already has tag among the person's boards:
// one open here, or one any of the person's devices made or joined
// (board.joined) that hasn't opened or folded here yet. "" when none.
func (c *Core) tagHolder(tag string) string {
	for _, other := range c.Boards() {
		if b, err := c.board(other); err == nil && b.verifier.Membership().Tag == tag {
			return other
		}
	}
	forgot := map[string]bool{}
	joined := map[string]string{}
	_ = c.eachRow(context.Background(), store.Filter{Kinds: []string{store.KindBoardJoined, store.KindBoardForgot}}, func(ev store.Event) {
		switch ev.Kind {
		case store.KindBoardJoined:
			var r boardJoinedRow
			if json.Unmarshal(ev.Payload, &r) == nil && r.Tag == tag {
				joined[r.BoardID] = r.Tag
				delete(forgot, r.BoardID)
			}
		case store.KindBoardForgot:
			var r boardForgotRow
			if json.Unmarshal(ev.Payload, &r) == nil {
				forgot[r.BoardID] = true
			}
		}
	})
	for id := range joined {
		if !forgot[id] {
			return id
		}
	}
	return ""
}

// now is the Core's clock.
func (c *Core) now() time.Time {
	if c.opts.Now != nil {
		return c.opts.Now()
	}
	return time.Now()
}

// RenameBoard renames a board (§4.2: owners only), with board.renamed
// signed by this person's key.
func (c *Core) RenameBoard(ctx context.Context, boardID, name string) error {
	name = strings.TrimSpace(name)
	if !identity.ValidBoardName(name) {
		return fmt.Errorf("%w: a board name is 1 to %d characters", coord.ErrInvalid, identity.MaxBoardName)
	}
	b, err := c.board(boardID)
	if err != nil {
		return err
	}
	me := c.chain.PersonID()
	if !b.verifier.Membership().Owner(me) {
		return fmt.Errorf("%w: only an owner renames board %s", coord.ErrNotAllowed, boardID)
	}
	epoch := b.verifier.Epoch(me)
	if epoch != c.ident.Current().Epoch {
		return fmt.Errorf("%w: this device's person key isn't the board's newest yet", coord.ErrNotAllowed)
	}
	row, err := c.ident.BoardRenamedRow(epoch, name, c.now())
	if err != nil {
		return err
	}
	_, err = b.store.Append(ctx, store.PersonActorPrefix+me, store.KindBoardRenamed, row)
	return err
}

// BoardInfo is one board as this device folds it, for the Boards dialog
// (step 4) and ?detail=1.
type BoardInfo struct {
	BoardID string                 `json:"boardId"`
	Name    string                 `json:"name"`
	Tag     string                 `json:"tag"`
	Creator string                 `json:"creator"`
	Members []identity.BoardPerson `json:"members"`
	// Conflicts are board.* rows the fold ignored, with why (§4.2).
	Conflicts []identity.Conflict `json:"conflicts,omitempty"`
	// Epoch is the board secret's epoch by the fold; SecretEpoch the
	// newest this device holds a secret for.
	Epoch       int `json:"epoch"`
	SecretEpoch int `json:"secretEpoch"`
}

// BoardInfo reports boardID's membership and secret epoch.
func (c *Core) BoardInfo(boardID string) (BoardInfo, error) {
	b, err := c.board(boardID)
	if err != nil {
		return BoardInfo{}, err
	}
	m := b.verifier.Membership()
	return BoardInfo{BoardID: boardID, Name: m.Name, Tag: m.Tag, Creator: m.Creator, Members: m.Members,
		Conflicts: m.Conflicts, Epoch: b.verifier.BoardEpoch(), SecretEpoch: b.secrets.Epoch()}, nil
}

// rotateSecret makes this device's secret for the board's epoch when it
// holds none yet (§6.1): at board.created, and after a removal, a leave or
// a member device revoke folds. Only a device of a current owner, or of a
// member whose own device was revoked, rotates. Sending it to the other
// member devices is step 5's.
func (c *Core) rotateSecret(b *boardLog) error {
	b.idMu.Lock()
	defer b.idMu.Unlock()
	target := b.verifier.BoardEpoch()
	if target == 0 || b.secrets.Epoch() >= target {
		return nil
	}
	if _, cut := b.verifier.Cut(c.device.ID); cut {
		return nil // the device a rotation shuts out makes no secret for it
	}
	me := c.chain.PersonID()
	if pin, _ := b.verifier.Pinned(me); target == 1 && pin.RootDeviceID != c.device.ID {
		// The board's first secret is its creating device's (§6.1); the
		// person's other devices get it in a board.secret frame (step 5)
		// rather than each making a different one.
		return nil
	}
	m := b.verifier.Membership()
	may := m.Owner(me)
	if !may && m.Active(me) {
		for _, d := range b.verifier.Devices(me) {
			may = may || d.Revoked
		}
	}
	if !may {
		return nil
	}
	return b.secrets.Mint(target, c.device.ID)
}

// openBoard opens (creating) one board log and its coord, and starts
// following it. create pins this person on it with this device as the
// board root.
func (c *Core) openBoard(ctx context.Context, id string, create bool, pin *identity.MemberPin) (*boardLog, error) {
	if !validBoardID(id) {
		return nil, fmt.Errorf("core: %q is not a board id", id)
	}
	c.boards.mu.Lock()
	defer c.boards.mu.Unlock()
	if b := c.boards.logs[id]; b != nil {
		return b, nil
	}
	secrets, err := identity.OpenBoardSecrets(filepath.Join(c.opts.DataDir, BoardsDir, id+".secrets"), id, c.ident.Keystore())
	if err != nil {
		return nil, fmt.Errorf("board secrets: %w", err)
	}
	v := identity.NewBoardVerifierFor(id)
	st, err := store.OpenWith(ctx, filepath.Join(c.opts.DataDir, BoardsDir, id+".db"), c.device.ID, hlc.New(c.opts.Now),
		store.OpenOptions{Signer: c.ident, Verifier: v, Now: c.opts.Now, Board: true})
	if err != nil {
		return nil, fmt.Errorf("open board log: %w", err)
	}
	b := &boardLog{id: id, store: st, verifier: v, secrets: secrets, cards: map[string]boardCard{}, keys: map[string]string{}, byID: map[string]string{}}
	secrets.SetMakers(func(dev string) bool {
		_, certified := v.Cert(dev)
		_, cut := v.Cut(dev)
		return certified && !cut
	})
	if err := c.pinBoard(ctx, b, create, pin); err != nil {
		st.Close()
		return nil, err
	}
	if err := b.load(ctx, c.store); err != nil {
		st.Close()
		return nil, err
	}
	if err := c.keepCurrent(ctx, b); err != nil {
		// The board still opens: its rows verify as far as they can, and
		// the next identity change retries.
		log.Printf("board %s: identity rows: %v", id, err)
	}
	if err := c.rotateSecret(b); err != nil {
		log.Printf("board %s: secret: %v", id, err)
	}
	me := store.PersonActorPrefix + c.chain.PersonID()
	if b.coord, err = coord.New(ctx, st, boardRoster{c, b}, coord.Options{Now: c.opts.Now, Board: true, Person: me,
		IsOwner:      func(k string) bool { return k == store.PersonActorPrefix+c.chain.PersonID() },
		Revoked:      func(dev string) bool { _, revoked := v.Cut(dev); return revoked },
		KeyPerson:    v.KeyPerson,
		DevicePerson: c.boardDevicePerson(v),
		Tag:          func() string { return v.Membership().Tag },
		BoardOwner:   func(p string) bool { return v.Membership().Owner(p) }}); err != nil {
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
	bctx, cancel := context.WithCancel(fctx)
	b.cancel = cancel
	b.wg.Add(3)
	go func() { defer b.wg.Done(); b.coord.Follow(bctx) }()
	go func() { defer b.wg.Done(); c.followBoard(bctx, b) }()
	go func() { defer b.wg.Done(); c.relayChanges(bctx, b) }()
	c.boards.follow.Do(func() {
		c.boards.wg.Add(1)
		go func() { defer c.boards.wg.Done(); c.followIdentity(fctx) }()
	})
	if c.mesh != nil {
		c.mesh.KickBoards()
	}
	return b, nil
}

// closeBoards waits for the boards' goroutines (the Core's context is
// already canceled) and closes their logs.
func (c *Core) closeBoards() {
	c.boards.mu.Lock()
	logs := make([]*boardLog, 0, len(c.boards.logs))
	for _, b := range c.boards.logs {
		logs = append(logs, b)
	}
	c.boards.mu.Unlock()
	done := make(chan struct{})
	go func() {
		c.boards.wg.Wait()
		for _, b := range logs {
			b.wg.Wait()
		}
		close(done)
	}()
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

// load folds the log's agent cards and the person's board_cards for it:
// the private rows (priv, the Core's private store) and this device's
// older local ones.
func (b *boardLog) load(ctx context.Context, priv *store.Store) error {
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
	if priv != nil {
		f := store.Filter{Kinds: []string{store.KindBoardCardOf}, Limit: store.MaxListLimit}
		for {
			evs, err := priv.List(ctx, f)
			if err != nil {
				return err
			}
			keys = append(keys, evs...)
			if len(evs) < f.Limit {
				break
			}
			f.After = evs[len(evs)-1].Cursor()
		}
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	for _, ev := range keys {
		b.foldKeyLocked(ev)
	}
	return nil
}

// foldKeyLocked folds one board_cards row, private or local, if it is
// this board's. b.mu is held.
func (b *boardLog) foldKeyLocked(ev store.Event) {
	var r cardKeyRow
	if json.Unmarshal(ev.Payload, &r) != nil || r.CardID == "" || r.AgentKey == "" {
		return
	}
	if ev.Kind == store.KindBoardCardOf && r.BoardID != b.id {
		return
	}
	b.addKey(r.CardID, r.AgentKey)
}

// foldCardKey folds a private board_cards row that reached this device
// into its board, when that board is open here.
func (c *Core) foldCardKey(ev store.Event) {
	var r cardKeyRow
	if json.Unmarshal(ev.Payload, &r) != nil {
		return
	}
	b, err := c.board(r.BoardID)
	if err != nil {
		return
	}
	b.mu.Lock()
	b.foldKeyLocked(ev)
	b.mu.Unlock()
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
// b's log, rebuilding them on a resync, and rotates the board secret when
// a membership or identity row moves the board's epoch.
func (c *Core) followBoard(ctx context.Context, b *boardLog) {
	for ev := range b.store.Subscribe(ctx) {
		if c.mesh != nil {
			c.mesh.KickBoards() // the person's other devices pull it now
		}
		if ev.Seq == 0 || store.IsBoardIdentityKind(ev.Kind) {
			if err := c.rotateSecret(b); err != nil && ctx.Err() == nil {
				log.Printf("board %s: secret: %v", b.id, err)
			}
		}
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
			_ = b.load(ctx, c.store)
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
		if len(keys) > 0 {
			// A write to a private card that was shared to a board is
			// refused, never rerouted (§5.2): its coord comes back too,
			// for callers that only read.
			if err := c.sharedRefusal(taskID); err != nil {
				return c.coord, keys, err
			}
		}
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
// holds a user: key; an agent on any of the person's devices (MeshAgents)
// becomes agent:<cardId>, its card written the first time it touches the
// board, from whichever device that is; "irori" and keys already in board
// form are kept. Anything else is refused: a made-up user: key is no one,
// and an agent the roster doesn't have is not the person's.
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
	if owner.DeviceOf(agent) == "" {
		return "", fmt.Errorf("%w: %q is not an agent of this person", coord.ErrNotAllowed, agent)
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
// private board_cards row that says which session it is (§2.4), which
// reaches the person's other devices.
func (c *Core) cardFor(ctx context.Context, b *boardLog, agent string) (string, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if id := b.keys[agent]; id != "" {
		return store.AgentActorPrefix + id, nil
	}
	var card *presence.AgentCard
	for _, a := range c.MeshAgents() {
		if a.Key == agent {
			card = &a
			break
		}
	}
	if card == nil {
		return "", fmt.Errorf("%w: no agent %q on the person's devices", coord.ErrNotFound, agent)
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
	// The mapping first: a card whose session nobody knows is worse than
	// a mapping to a card that failed to append (it is retried).
	if _, err := c.store.Append(ctx, "irori", store.KindBoardCardOf, cardKeyRow{BoardID: b.id, CardID: id, AgentKey: agent}); err != nil {
		return "", err
	}
	if _, err := b.store.Append(ctx, store.AgentActorPrefix+id, store.KindAgentCard, row); err != nil {
		return "", err
	}
	b.addKey(id, agent)
	b.cards[id] = row
	return store.AgentActorPrefix + b.keys[agent], nil
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
// the log's agent cards, keyed agent:<cardId>. A card whose session is on
// this device is as live as the session (missing when the session is).
// Only that device says a session went (v0.4 §6.0: a holder on an offline
// device keeps its cards), and a board key names no device, so a card of
// another device's session, or of nobody's this device knows, never reads
// as gone here: it shows its session's status while that is live, else
// its card row's.
type boardRoster struct {
	c *Core
	b *boardLog
}

func (r boardRoster) Agents() []presence.AgentCard {
	live := map[string]presence.AgentCard{}
	for _, a := range r.c.MeshAgents() {
		live[a.Key] = a
	}
	self := r.c.device.ID
	r.b.mu.Lock()
	defer r.b.mu.Unlock()
	out := make([]presence.AgentCard, 0, len(r.b.cards))
	for id, card := range r.b.cards {
		a := presence.AgentCard{Key: store.AgentActorPrefix + id, Name: card.Name, Agent: card.Kind, Status: card.Status}
		if agent := r.b.byID[id]; agent != "" {
			s, ok := live[agent]
			switch {
			case owner.DeviceOf(agent) == self && !ok:
				continue
			case owner.DeviceOf(agent) == self, ok && s.Status != "gone":
				a.Status, a.UpdatedAt = s.Status, s.UpdatedAt
			}
		}
		out = append(out, a)
	}
	return out
}

// pinBoard pins b's Verifier from its board.pin rows (§3). A board with
// none is pinned to this person, rooted at this device, when this device
// creates it or wrote every row it holds (a board from before pins); a
// board holding another device's rows and no pin is left unpinned, so
// nothing in it verifies, and says so.
//
// given, when set, is the creator's pin from a board.joined row one of this
// person's devices wrote (boards_own.go): a board this device opens
// because another of its person's devices made or joined it.
func (c *Core) pinBoard(ctx context.Context, b *boardLog, create bool, given *identity.MemberPin) error {
	evs, err := b.store.List(ctx, store.Filter{Kinds: []string{store.KindBoardPin}, Local: true, Limit: store.MaxListLimit})
	if err != nil {
		return err
	}
	pinned := false
	for _, ev := range evs {
		var p identity.MemberPin
		if json.Unmarshal(ev.Payload, &p) != nil {
			continue
		}
		if err := b.verifier.Pin(p); err != nil {
			log.Printf("board %s: %v", b.id, err)
			continue
		}
		pinned = true
	}
	if !pinned && given != nil {
		if err := b.verifier.Pin(*given); err != nil {
			return err
		}
		if _, err := b.store.AppendLocal(ctx, "irori", store.KindBoardPin, *given); err != nil {
			return err
		}
		pinned = true
	}
	if !pinned {
		vec, err := b.store.Vector(ctx)
		if err != nil {
			return err
		}
		for dev := range vec {
			if dev != c.device.ID && !create {
				log.Printf("board %s: holds rows from %s but pins nobody; nothing in it verifies", b.id, dev)
				return nil
			}
		}
		p := c.ident.BoardPin()
		if _, err := b.store.AppendLocal(ctx, "irori", store.KindBoardPin, p); err != nil {
			return err
		}
		if err := b.verifier.Pin(p); err != nil {
			return err
		}
	}
	// The rows already observed fold again, now pinned, and the store
	// applies their cuts.
	b.store.ReloadVerifier(ctx)
	return nil
}

// boardDevicePerson is a board coord's DevicePerson: the member v
// certified a device to, and this person for this device even before its
// board cert folds (its own rows are this person's).
func (c *Core) boardDevicePerson(v *identity.BoardVerifier) func(string) string {
	return func(dev string) string {
		if p, ok := v.PersonOf(dev); ok {
			return p
		}
		if dev == c.device.ID {
			return c.chain.PersonID()
		}
		return ""
	}
}

// followIdentity keeps every board's member.* rows current as the private
// log's person.* rows change: a device paired, revoked, or the key
// rotated (§3, "Keeping boards current").
func (c *Core) followIdentity(ctx context.Context) {
	for ev := range c.store.Subscribe(ctx) {
		if ev.Seq != 0 && !store.IsPersonKind(ev.Kind) {
			continue
		}
		c.boards.mu.Lock()
		logs := make([]*boardLog, 0, len(c.boards.logs))
		for _, b := range c.boards.logs {
			logs = append(logs, b)
		}
		c.boards.mu.Unlock()
		for _, b := range logs {
			if err := c.keepCurrent(ctx, b); err != nil && ctx.Err() == nil {
				log.Printf("board %s: identity rows: %v", b.id, err)
			}
		}
	}
}

// keepCurrent writes into b what this person's identity in it lacks (§3):
// this device's self-cert when it is the board root, then, from a device
// whose board cert counts, each rotation this device made (only the
// rotating device can, it alone signed prevSig), the revoke of each
// device the private chain revoked with a cut over b's own stream, a cert
// for each certified device of this person, and the profile. Rows are
// signed for the board's newest epoch of this person, so they are written
// only while this device's key is that epoch's.
func (c *Core) keepCurrent(ctx context.Context, b *boardLog) error {
	b.idMu.Lock()
	defer b.idMu.Unlock()
	v, self := b.verifier, c.device.ID
	me := c.ident.Current()
	pin, ok := v.Pinned(me.ID)
	if !ok {
		return nil
	}
	if m := v.Membership(); m.Created && !m.Active(me.ID) {
		return nil // removed or left: nothing more of ours counts here
	}
	now := time.Now
	if c.opts.Now != nil {
		now = c.opts.Now
	}
	role := identity.RoleDesktop
	if cert, ok := c.chain.Cert(self); ok && cert.Role != "" {
		role = cert.Role
	}
	if _, ok := v.Cert(self); !ok {
		if pin.RootDeviceID != self {
			return nil // another device of this person certifies this one
		}
		add, err := c.ident.MemberCertFor(pin.PersonEpoch, self, hex.EncodeToString(c.ident.DevicePub()), role, now())
		if err != nil {
			return fmt.Errorf("self-cert: %w", err)
		}
		if _, err := b.store.Append(ctx, "irori", store.KindMemberDeviceAdded, add); err != nil {
			return err
		}
		if _, ok := v.Cert(self); !ok {
			return errors.New("the self-cert did not certify this device")
		}
	}
	if _, revoked := v.Cut(self); revoked {
		return nil
	}
	for {
		next := v.Epoch(me.ID) + 1
		rot, ok, err := c.ownRotation(ctx, next)
		if err != nil || !ok {
			if err != nil {
				return err
			}
			break
		}
		if wrote, err := b.wroteRotation(ctx, self, next); err != nil || wrote {
			return err // written before and it didn't count (it lost a tie): not again
		}
		if _, err := b.store.Append(ctx, "irori", store.KindMemberKeyRotated, rot); err != nil {
			return err
		}
		if v.Epoch(me.ID) < next {
			break // it didn't count; don't write it again
		}
	}
	epoch := v.Epoch(me.ID)
	if epoch != c.ident.Current().Epoch || !v.Writable(me.ID) {
		return nil // this key isn't the board's newest for this person
	}
	devs := c.chain.Devices()
	vec, err := b.store.Vector(ctx)
	if err != nil {
		return err
	}
	for _, d := range devs {
		dev := d.Cert.DeviceID
		if _, held := v.Cert(dev); !d.Revoked || !held || dev == self {
			continue
		}
		if _, cut := v.Cut(dev); cut {
			continue
		}
		cut, hash := vec[dev], ""
		if cut > 0 {
			evs, err := b.store.List(ctx, store.Filter{DeviceID: dev, AfterSeq: cut - 1, Limit: 1})
			if err != nil {
				return err
			}
			if len(evs) == 1 && evs[0].Seq == cut {
				hash = store.RowHash(evs[0])
			}
		}
		rev, err := c.ident.MemberRevokeOf(epoch, dev, cut, hash, now())
		if err != nil {
			return err
		}
		if _, err := b.store.Append(ctx, "irori", store.KindMemberDeviceRevoked, rev); err != nil {
			return err
		}
	}
	for _, d := range devs {
		dev := d.Cert.DeviceID
		if _, held := v.Cert(dev); d.Revoked || held {
			continue
		}
		add, err := c.ident.MemberCertFor(epoch, dev, d.Cert.DevicePub, d.Cert.Role, now())
		if err != nil {
			return err
		}
		if _, err := b.store.Append(ctx, "irori", store.KindMemberDeviceAdded, add); err != nil {
			return err
		}
	}
	if _, ok := v.Profile(me.ID); !ok {
		prof, err := c.ident.MemberProfileRow(epoch)
		if err != nil {
			return err
		}
		if _, err := b.store.Append(ctx, "irori", store.KindMemberProfile, prof); err != nil {
			return err
		}
	}
	return nil
}

// wroteRotation reports whether dev already wrote a member.key.rotated to
// epoch in b.
func (b *boardLog) wroteRotation(ctx context.Context, dev string, epoch int) (bool, error) {
	f := store.Filter{Kinds: []string{store.KindMemberKeyRotated}, DeviceID: dev, Limit: store.MaxListLimit}
	for {
		evs, err := b.store.List(ctx, f)
		if err != nil {
			return false, err
		}
		for _, ev := range evs {
			var r identity.MemberKeyRotated
			if json.Unmarshal(ev.Payload, &r) == nil && r.PersonEpoch == epoch {
				return true, nil
			}
		}
		if len(evs) < f.Limit {
			return false, nil
		}
		f.After = evs[len(evs)-1].Cursor()
	}
}

// ownRotation is the private person.key.rotated row this device wrote for
// epoch, as a board's member.key.rotated. ok is false when this device
// made no such rotation, or made it before v0.5 (no prevSig).
func (c *Core) ownRotation(ctx context.Context, epoch int) (identity.MemberKeyRotated, bool, error) {
	f := store.Filter{Kinds: []string{store.KindPersonKeyRotated}, DeviceID: c.device.ID, Limit: store.MaxListLimit}
	me := c.chain.PersonID()
	for {
		evs, err := c.store.List(ctx, f)
		if err != nil {
			return identity.MemberKeyRotated{}, false, err
		}
		for _, ev := range evs {
			var r identity.KeyRotated
			if json.Unmarshal(ev.Payload, &r) != nil || r.PersonID != me || r.Epoch != epoch {
				continue
			}
			if m, ok := identity.MemberRotation(r); ok && m.By == c.device.ID {
				return m, true, nil
			}
		}
		if len(evs) < f.Limit {
			return identity.MemberKeyRotated{}, false, nil
		}
		f.After = evs[len(evs)-1].Cursor()
	}
}

// BoardIdentity is what a board's Verifier holds about one member, for
// tests and ?detail=1.
type BoardIdentity struct {
	PersonID string                 `json:"personId"`
	Epoch    int                    `json:"epoch"`
	Name     string                 `json:"name,omitempty"`
	Devices  []identity.BoardDevice `json:"devices"`
}

// BoardMember reports this device's view of personID on boardID.
func (c *Core) BoardMember(boardID, personID string) (BoardIdentity, error) {
	b, err := c.board(boardID)
	if err != nil {
		return BoardIdentity{}, err
	}
	name, _ := b.verifier.Profile(personID)
	return BoardIdentity{PersonID: personID, Epoch: b.verifier.Epoch(personID), Name: name, Devices: b.verifier.Devices(personID)}, nil
}

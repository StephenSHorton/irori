package core

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/scrub"
	"github.com/StephenSHorton/irori/internal/store"
)

// Shared boards, step 4 (design/v0.5-shared-boards.md §12): one person,
// several boards, on the person's own devices only.
//
//   - A board reaches the person's other devices through the private log:
//     CreateBoard writes board.joined {boardId, name, tag, creator}, and
//     every device of the person opens that board pinned to the creator
//     it names (§4.1 step 7, own devices only until invites). The board's
//     rows then sync between them on board/<id> streams of the private
//     mesh connection (mesh/boards.go), never on its sync channel. Step 5
//     moves them to each board's own topic (§6.3).
//   - Forget drops a board on this device: its file and secrets go, and a
//     device-local forgotten marker keeps an older board.joined from
//     bringing it back (§4.6). Leave forgets it on every device of the
//     person through a private board.forgot row. With nobody else on the
//     board there is no other member to wait for (§4.4's "Leaving…").
//   - Share to board writes a fresh task.created on a board with the
//     card's scrubbed fields and steps (§5.2), keeps a device-local
//     shared_copies row, closes the private card with a note naming the
//     shared copy, and refuses later writes to the private id.
//   - Withdraw cancels a shared card with withdrawn set (§5.3); its holders
//     here carry on in a new private card.
//   - Short refs carry the board's tag (`mlg#12`, B3), and TaskRef resolves
//     a ref across every log this device holds.

// boardJoinedRow is the private board.joined row.
type boardJoinedRow struct {
	BoardID string             `json:"boardId"`
	Name    string             `json:"name"`
	Tag     string             `json:"tag"`
	Creator identity.MemberPin `json:"creator"`
	At      time.Time          `json:"at"`
}

// boardForgotRow is the private board.forgot row: Leave, on every device.
type boardForgotRow struct {
	BoardID string    `json:"boardId"`
	At      time.Time `json:"at"`
}

// forgottenRow is the device-local forgotten marker (store.KindBoardForgotten).
type forgottenRow struct {
	BoardID string `json:"boardId"`
	UpTo    uint64 `json:"upTo"` // the newest board.joined HLC it forgot
}

// sharedCopy is one device-local shared_copies row (store.KindSharedCopy).
type sharedCopy struct {
	PrivateTaskID string `json:"privateTaskId"`
	BoardID       string `json:"boardId"`
	TaskID        string `json:"taskId"`
}

// ErrShared: a write to a private card that was shared to a board (§5.2).
var ErrShared = errors.New("core: the card was shared to a board")

var _ mesh.BoardLogs = (*Core)(nil)

// BoardIDs lists the boards open on this device (mesh.BoardLogs).
func (c *Core) BoardIDs() []string { return c.Boards() }

// BoardStore is board id's open log, or nil (mesh.BoardLogs).
func (c *Core) BoardStore(id string) *store.Store {
	b, err := c.board(id)
	if err != nil {
		return nil
	}
	return b.store
}

// followOwnBoards opens and forgets boards as the private log's
// board.joined and board.forgot rows arrive, and loads this device's
// shared copies.
func (c *Core) followOwnBoards(ctx context.Context) {
	sub := c.store.Subscribe(ctx)
	c.loadCopies(ctx)
	c.reconcileBoards(ctx)
	for ev := range sub {
		if ev.Seq == 0 || ev.Kind == store.KindBoardJoined || ev.Kind == store.KindBoardForgot {
			c.reconcileBoards(ctx)
		}
	}
}

// reconcileBoards brings the open boards in line with the private log: a
// board.joined opens its board (pinned to the creator it names) unless a
// newer board.forgot or this device's forgotten marker covers it; a
// board.forgot newer than every board.joined forgets it here.
func (c *Core) reconcileBoards(ctx context.Context) {
	c.boards.recMu.Lock()
	defer c.boards.recMu.Unlock()
	if ctx.Err() != nil {
		return
	}
	joined := map[string]struct {
		row boardJoinedRow
		at  hlc.Timestamp
	}{}
	forgot := map[string]hlc.Timestamp{}
	err := c.eachRow(ctx, store.Filter{Kinds: []string{store.KindBoardJoined, store.KindBoardForgot}}, func(ev store.Event) {
		switch ev.Kind {
		case store.KindBoardJoined:
			var r boardJoinedRow
			if json.Unmarshal(ev.Payload, &r) == nil && validBoardID(r.BoardID) && ev.HLC >= joined[r.BoardID].at {
				joined[r.BoardID] = struct {
					row boardJoinedRow
					at  hlc.Timestamp
				}{r, ev.HLC}
			}
		case store.KindBoardForgot:
			var r boardForgotRow
			if json.Unmarshal(ev.Payload, &r) == nil && validBoardID(r.BoardID) && ev.HLC > forgot[r.BoardID] {
				forgot[r.BoardID] = ev.HLC
			}
		}
	})
	if err != nil {
		return
	}
	marks := c.forgottenMarks(ctx)
	for id, f := range forgot {
		if j, ok := joined[id]; ok && j.at > f {
			continue
		}
		if _, err := c.board(id); err == nil {
			if err := c.forgetBoard(ctx, id, uint64(joined[id].at)); err != nil {
				log.Printf("board %s: forget: %v", id, err)
			}
		}
	}
	for id, j := range joined {
		if f, ok := forgot[id]; ok && f >= j.at {
			continue
		}
		if up, ok := marks[id]; ok && up >= uint64(j.at) {
			continue
		}
		if _, err := c.board(id); err == nil {
			continue
		}
		pin := j.row.Creator
		if _, err := c.openBoard(ctx, id, false, &pin); err != nil {
			log.Printf("board %s: open from board.joined: %v", id, err)
			continue
		}
		c.boardsMoved()
	}
}

// eachRow calls fn for every private-log row f matches, in log order.
func (c *Core) eachRow(ctx context.Context, f store.Filter, fn func(store.Event)) error {
	f.Limit = store.MaxListLimit
	for {
		evs, err := c.store.List(ctx, f)
		if err != nil {
			return err
		}
		for _, ev := range evs {
			fn(ev)
		}
		if len(evs) < f.Limit {
			return nil
		}
		f.After = evs[len(evs)-1].Cursor()
	}
}

// forgottenMarks is this device's forgotten list: boardId -> upTo.
func (c *Core) forgottenMarks(ctx context.Context) map[string]uint64 {
	out := map[string]uint64{}
	_ = c.eachRow(ctx, store.Filter{Kinds: []string{store.KindBoardForgotten}, Local: true}, func(ev store.Event) {
		var r forgottenRow
		if json.Unmarshal(ev.Payload, &r) == nil && r.BoardID != "" && r.UpTo >= out[r.BoardID] {
			out[r.BoardID] = r.UpTo
		}
	})
	return out
}

// ForgetBoard forgets a board on this device only (§4.6): its log and
// secrets are deleted here, and a later board.joined from another of the
// person's devices doesn't bring it back. Other devices keep it.
func (c *Core) ForgetBoard(ctx context.Context, boardID string) error {
	if _, err := c.board(boardID); err != nil {
		return err
	}
	c.boards.recMu.Lock()
	defer c.boards.recMu.Unlock()
	var upTo uint64
	_ = c.eachRow(ctx, store.Filter{Kinds: []string{store.KindBoardJoined}}, func(ev store.Event) {
		var r boardJoinedRow
		if json.Unmarshal(ev.Payload, &r) == nil && r.BoardID == boardID {
			upTo = max(upTo, uint64(ev.HLC))
		}
	})
	return c.forgetBoard(ctx, boardID, upTo)
}

// LeaveBoard takes the person off a board on every one of their devices
// (§4.4, §4.6) through a private board.forgot row. A board with other
// people on it is left with board.member.left, which comes with invites
// (stage 2); until then only a board of the person's own is left.
func (c *Core) LeaveBoard(ctx context.Context, boardID string) error {
	b, err := c.board(boardID)
	if err != nil {
		return err
	}
	me := c.chain.PersonID()
	for _, p := range b.verifier.Membership().Members {
		if p.PersonID != me && p.State == identity.MemberActive {
			return fmt.Errorf("%w: board %s has other people on it; leaving it is not built yet", coord.ErrNotAllowed, boardID)
		}
	}
	if _, err := c.store.Append(ctx, "irori", store.KindBoardForgot, boardForgotRow{BoardID: boardID, At: c.now()}); err != nil {
		return err
	}
	c.reconcileBoards(ctx)
	return nil
}

// forgetBoard closes board id, deletes its files and records the
// forgotten marker. recMu is held.
func (c *Core) forgetBoard(ctx context.Context, id string, upTo uint64) error {
	if _, err := c.store.AppendLocal(ctx, "irori", store.KindBoardForgotten, forgottenRow{BoardID: id, UpTo: upTo}); err != nil {
		return err
	}
	c.dropBoard(id)
	dir := filepath.Join(c.opts.DataDir, BoardsDir)
	var errs []error
	for _, name := range []string{id + ".db", id + ".db-wal", id + ".db-shm", id + ".secrets"} {
		if err := os.Remove(filepath.Join(dir, name)); err != nil && !errors.Is(err, os.ErrNotExist) {
			errs = append(errs, err)
		}
	}
	c.boardsMoved()
	return errors.Join(errs...)
}

// dropBoard closes board id's log and takes it out of the Core.
func (c *Core) dropBoard(id string) {
	c.boards.mu.Lock()
	b := c.boards.logs[id]
	delete(c.boards.logs, id)
	for t, on := range c.boards.taskOf {
		if on == id {
			delete(c.boards.taskOf, t)
		}
	}
	c.boards.mu.Unlock()
	if b == nil {
		return
	}
	if b.cancel != nil {
		b.cancel()
	}
	done := make(chan struct{})
	go func() { b.wg.Wait(); close(done) }()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
	}
	b.store.Close()
}

// SubscribeBoards delivers every board coord's changes, and a zero Change
// when a board opens or goes, until ctx ends (the window's pump).
func (c *Core) SubscribeBoards(ctx context.Context) <-chan coord.Change {
	ch := make(chan coord.Change, 64)
	c.boards.subMu.Lock()
	if c.boards.subs == nil {
		c.boards.subs = map[chan coord.Change]struct{}{}
	}
	c.boards.subs[ch] = struct{}{}
	c.boards.subMu.Unlock()
	go func() {
		<-ctx.Done()
		c.boards.subMu.Lock()
		delete(c.boards.subs, ch)
		close(ch) // tellBoards sends under subMu, so none is in flight
		c.boards.subMu.Unlock()
	}()
	return ch
}

func (c *Core) tellBoards(ch coord.Change) {
	c.boards.subMu.Lock()
	defer c.boards.subMu.Unlock()
	for s := range c.boards.subs {
		select {
		case s <- ch:
		default: // a slow reader redraws on the next one
		}
	}
}

// boardsMoved tells the window a board opened, went or was renamed.
func (c *Core) boardsMoved() { c.tellBoards(coord.Change{}) }

// relayChanges hands b's coord changes to SubscribeBoards until ctx ends.
func (c *Core) relayChanges(ctx context.Context, b *boardLog) {
	for ch := range b.coord.Subscribe(ctx) {
		c.tellBoards(ch)
	}
}

// BoardSummary is one board for the window's chips and the Boards dialog.
type BoardSummary struct {
	BoardID string `json:"boardId"`
	Name    string `json:"name"`
	Tag     string `json:"tag"`
	// People are the board's current members' names, this person first
	// ("Visible to: Stephen").
	People []string `json:"people"`
	Owner  bool     `json:"owner"`
	// Devices counts this person's devices certified on the board.
	Devices int `json:"devices"`
	// Cards counts its live cards.
	Cards int `json:"cards"`
	// Ready: its board.created counted here, so cards can go on it. A
	// board another device made shows up before its rows arrive.
	Ready bool `json:"ready"`
}

// BoardList is every board on this device, by name.
func (c *Core) BoardList() []BoardSummary {
	me := c.chain.PersonID()
	var out []BoardSummary
	for _, id := range c.Boards() {
		b, err := c.board(id)
		if err != nil {
			continue
		}
		m := b.verifier.Membership()
		s := BoardSummary{BoardID: id, Name: m.Name, Tag: m.Tag, Owner: m.Owner(me), Ready: m.Created, People: c.boardPeople(b)}
		if s.Name == "" {
			s.Name, s.Tag = c.joinedName(id)
		}
		for _, d := range b.verifier.Devices(me) {
			if !d.Revoked {
				s.Devices++
			}
		}
		for _, t := range b.coord.ListTasks(coord.TaskFilter{Open: true}) {
			if !t.Withdrawn {
				s.Cards++
			}
		}
		out = append(out, s)
	}
	sort.SliceStable(out, func(i, j int) bool { return strings.ToLower(out[i].Name) < strings.ToLower(out[j].Name) })
	return out
}

// joinedName is a board's name and tag from its board.joined row, for a
// board whose rows haven't arrived yet.
func (c *Core) joinedName(id string) (name, tag string) {
	_ = c.eachRow(context.Background(), store.Filter{Kinds: []string{store.KindBoardJoined}}, func(ev store.Event) {
		var r boardJoinedRow
		if json.Unmarshal(ev.Payload, &r) == nil && r.BoardID == id {
			name, tag = r.Name, r.Tag
		}
	})
	return name, tag
}

// boardPeople are b's current members' names, this person first.
func (c *Core) boardPeople(b *boardLog) []string {
	me := c.chain.PersonID()
	m := b.verifier.Membership()
	out := []string{}
	add := func(p string) {
		name, _ := b.verifier.Profile(p)
		if name == "" {
			if bp, ok := m.Person(p); ok {
				name = bp.Name
			}
		}
		if name == "" && p == me {
			name = c.ident.Current().Name
		}
		out = append(out, cmpOr(name, "someone"))
	}
	if m.Active(me) || !m.Created {
		add(me)
	}
	for _, p := range m.Members {
		if p.PersonID != me && p.State == identity.MemberActive {
			add(p.PersonID)
		}
	}
	return out
}

func cmpOr(a, b string) string {
	if a != "" {
		return a
	}
	return b
}

// BoardOf is the board a task lives on: its summary, or ok false for a
// private card.
func (c *Core) BoardOf(taskID string) (BoardSummary, bool) {
	id, _, err := c.Route(taskID)
	if err != nil || id == "" {
		return BoardSummary{}, false
	}
	for _, s := range c.BoardList() {
		if s.BoardID == id {
			return s, true
		}
	}
	return BoardSummary{}, false
}

// boardByTag is the open board tagged tag.
func (c *Core) boardByTag(tag string) (*boardLog, bool) {
	for _, id := range c.Boards() {
		if b, err := c.board(id); err == nil && b.verifier.Membership().Tag == tag {
			return b, true
		}
	}
	return nil, false
}

// TaskRef resolves what someone typed for a card across every log this
// device holds (§2.5, B3): "mlg#12" on the board tagged mlg; a bare "#12"
// only when exactly one log has it, else coord.ErrAmbiguous naming each;
// a full id, with or without "task:", as given (tools.TaskRouter).
func (c *Core) TaskRef(ref string) (string, error) {
	v := strings.TrimSpace(ref)
	if k, id, ok := strings.Cut(v, ":"); ok && strings.EqualFold(strings.TrimSpace(k), "task") {
		v = strings.TrimSpace(id)
	}
	tag, _, ok := coord.ParseRef(v)
	if !ok {
		return c.coord.TaskRef(ref)
	}
	if tag != "" {
		b, ok := c.boardByTag(tag)
		if !ok {
			return "", fmt.Errorf("%w: no board here is tagged %q", coord.ErrNotFound, tag)
		}
		return b.coord.TaskRef(v)
	}
	coords := []*coord.Coord{c.coord}
	for _, id := range c.Boards() {
		if b, err := c.board(id); err == nil {
			coords = append(coords, b.coord)
		}
	}
	var hits []coord.Task
	for _, co := range coords {
		id, err := co.TaskRef(v)
		if errors.Is(err, coord.ErrAmbiguous) {
			return "", err
		}
		if err != nil {
			continue
		}
		if t, ok := co.GetTask(id); ok {
			hits = append(hits, t)
		}
	}
	switch len(hits) {
	case 0:
		return c.coord.TaskRef(ref)
	case 1:
		return hits[0].ID, nil
	}
	names := make([]string, 0, len(hits))
	for _, t := range hits {
		names = append(names, fmt.Sprintf("%s %q (task:%s)", t.ShortRef(), oneLine(t.Title, 60), t.ID))
	}
	return "", fmt.Errorf("%w: #%s is on more than one board here: %s; say which", coord.ErrAmbiguous, strings.TrimPrefix(v, "#"), strings.Join(names, ", "))
}

func oneLine(s string, n int) string {
	s = strings.Join(strings.Fields(s), " ")
	if r := []rune(s); len(r) > n {
		s = string(r[:n-1]) + "…"
	}
	return s
}

// loadCopies reads this device's shared_copies rows.
func (c *Core) loadCopies(ctx context.Context) {
	copies := map[string]sharedCopy{}
	_ = c.eachRow(ctx, store.Filter{Kinds: []string{store.KindSharedCopy}, Local: true}, func(ev store.Event) {
		var r sharedCopy
		if json.Unmarshal(ev.Payload, &r) == nil && r.PrivateTaskID != "" {
			copies[r.PrivateTaskID] = r
		}
	})
	c.boards.mu.Lock()
	defer c.boards.mu.Unlock()
	if c.boards.copies == nil {
		c.boards.copies = map[string]sharedCopy{}
	}
	for k, v := range copies {
		c.boards.copies[k] = v
	}
}

// SharedCopy is where a private card was shared to, when it was: the
// board and the shared card, and its ref there ("mlg#4").
type SharedCopy struct {
	BoardID   string `json:"boardId"`
	BoardName string `json:"boardName"`
	TaskID    string `json:"taskId"`
	Ref       string `json:"ref"`
	Withdrawn bool   `json:"withdrawn,omitempty"`
}

// SharedCopyOf reports where private card taskID was shared to.
func (c *Core) SharedCopyOf(taskID string) (SharedCopy, bool) {
	c.boards.mu.Lock()
	sc, ok := c.boards.copies[taskID]
	c.boards.mu.Unlock()
	if !ok {
		return SharedCopy{}, false
	}
	out := SharedCopy{BoardID: sc.BoardID, TaskID: sc.TaskID, Ref: "task:" + sc.TaskID}
	if b, err := c.board(sc.BoardID); err == nil {
		out.BoardName = b.verifier.Membership().Name
		if t, ok := b.coord.GetTask(sc.TaskID); ok {
			out.Ref, out.Withdrawn = t.ShortRef(), t.Withdrawn
		}
	}
	return out, true
}

// sharedRefusal is ErrShared for a write to a private card that was shared
// to a board and not withdrawn, naming the shared card and who sees it.
func (c *Core) sharedRefusal(taskID string) error {
	sc, ok := c.SharedCopyOf(taskID)
	if !ok || sc.Withdrawn {
		return nil
	}
	b, err := c.board(sc.BoardID)
	if err != nil {
		return nil // the board is gone from this device: nothing to refuse for
	}
	return fmt.Errorf("%w: it was shared to %s as %s (visible to: %s), and this private card is closed; write to %s instead",
		ErrShared, cmpOr(sc.BoardName, "a board"), sc.Ref, strings.Join(c.boardPeople(b), ", "), sc.Ref)
}

// SharePreview is exactly what Share to board would write (§5.2): the
// card's fields after scrub and the stricter pass, and who will see it.
type SharePreview struct {
	BoardID   string   `json:"boardId"`
	BoardName string   `json:"boardName"`
	Title     string   `json:"title"`
	Goal      string   `json:"goal"`
	DoneWhen  string   `json:"doneWhen"`
	Steps     []string `json:"steps"`
	Links     []string `json:"links"`
	Repo      string   `json:"repo"`
	VisibleTo []string `json:"visibleTo"`
	// Dropped says what scrub took out, one line each ("a path in the
	// goal"), so the person sees why the text differs.
	Dropped []string `json:"dropped"`
}

// shareable is the private card taskID, if it can be shared.
func (c *Core) shareable(taskID string) (coord.Task, error) {
	t, ok := c.coord.GetTask(taskID)
	switch {
	case !ok:
		if id, _, err := c.Route(taskID); err == nil && id != "" {
			return coord.Task{}, fmt.Errorf("%w: the card is already on a board", coord.ErrInvalid)
		}
		return coord.Task{}, fmt.Errorf("%w: task %q", coord.ErrNotFound, taskID)
	case coord.IsTerminal(t.State):
		return coord.Task{}, fmt.Errorf("%w: %s is %s; share a card that is still in play", coord.ErrBadState, t.ShortRef(), t.State)
	case t.Ask != nil:
		return coord.Task{}, fmt.Errorf("%w: an ask isn't shared; its outcome card can be", coord.ErrInvalid)
	case t.Parent != "" || len(t.Subtasks) > 0:
		return coord.Task{}, fmt.Errorf("%w: a card with subtasks, or a subtask, isn't shared yet", coord.ErrInvalid)
	}
	if _, ok := c.SharedCopyOf(taskID); ok {
		return coord.Task{}, fmt.Errorf("%w: %s was shared already", coord.ErrInvalid, t.ShortRef())
	}
	return t, nil
}

// PreviewShare is SharePreview for private card taskID on boardID.
func (c *Core) PreviewShare(taskID, boardID string) (SharePreview, error) {
	t, err := c.shareable(taskID)
	if err != nil {
		return SharePreview{}, err
	}
	b, err := c.board(boardID)
	if err != nil {
		return SharePreview{}, err
	}
	m := b.verifier.Membership()
	if !m.Created || !m.Active(c.chain.PersonID()) {
		return SharePreview{}, fmt.Errorf("%w: board %s isn't ready on this device yet", coord.ErrBadState, cmpOr(m.Name, boardID))
	}
	p := SharePreview{BoardID: boardID, BoardName: m.Name, Steps: []string{}, Links: []string{}, Dropped: []string{}, VisibleTo: c.boardPeople(b)}
	clean := func(what, in string) string {
		s := c.scrubber.String(in)
		if s != in {
			p.Dropped = append(p.Dropped, "a secret in the "+what)
		}
		if st := scrub.Strict(s); st != s {
			p.Dropped = append(p.Dropped, "a path in the "+what)
			s = st
		}
		return s
	}
	p.Title = clean("title", t.Title)
	p.Goal = clean("goal", t.Objective)
	p.DoneWhen = clean("done-when", t.DoneCriteria)
	for _, s := range t.Steps {
		p.Steps = append(p.Steps, clean("steps", s.Text))
	}
	for _, l := range t.Links {
		switch {
		case l.Kind == coord.LinkPath && scrub.StrictDropped(l.Ref):
			p.Dropped = append(p.Dropped, "a link to a path on this machine")
		case c.scrubber.String(l.Ref) != l.Ref:
			p.Dropped = append(p.Dropped, "a link with a secret in it")
		default:
			p.Links = append(p.Links, l.Ref)
		}
	}
	if t.RepoKey != "" && !scrub.StrictDropped(t.RepoKey) {
		p.Repo = t.RepoKey
	}
	p.Dropped = dedupe(p.Dropped)
	return p, nil
}

func dedupe(in []string) []string {
	seen := map[string]bool{}
	out := in[:0]
	for _, s := range in {
		if !seen[s] {
			seen[s] = true
			out = append(out, s)
		}
	}
	return out
}

// ShareToBoard shares private card taskID to boardID as the person (§5.2):
// a fresh task.created with the preview's fields and steps, the
// shared_copies row, and the private card closed with a note naming the
// shared copy. Its comments, notes and blockers stay private. The window
// shows the preview first (PreviewShare).
func (c *Core) ShareToBoard(ctx context.Context, taskID, boardID string) (coord.Task, error) {
	p, err := c.PreviewShare(taskID, boardID)
	if err != nil {
		return coord.Task{}, err
	}
	b, err := c.board(boardID)
	if err != nil {
		return coord.Task{}, err
	}
	steps := make([]coord.Step, 0, len(p.Steps))
	priv, _ := c.coord.GetTask(taskID)
	for i, s := range p.Steps {
		st := coord.Step{Text: s}
		if i < len(priv.Steps) {
			st.Done = priv.Steps[i].Done
		}
		steps = append(steps, st)
	}
	me := store.PersonActorPrefix + c.chain.PersonID()
	shared, err := b.coord.CreateTask(ctx, me, coord.NewTask{Title: p.Title, Objective: p.Goal, DoneCriteria: p.DoneWhen,
		RepoKey: p.Repo, Links: p.Links, Steps: steps})
	if err != nil {
		return coord.Task{}, err
	}
	c.boards.mu.Lock()
	c.boards.taskOf[shared.ID] = boardID
	c.boards.mu.Unlock()
	row := sharedCopy{PrivateTaskID: taskID, BoardID: boardID, TaskID: shared.ID}
	if _, err := c.store.AppendLocal(ctx, "irori", store.KindSharedCopy, row); err != nil {
		return shared, err
	}
	c.boards.mu.Lock()
	if c.boards.copies == nil {
		c.boards.copies = map[string]sharedCopy{}
	}
	c.boards.copies[taskID] = row
	c.boards.mu.Unlock()
	// The private card closes; its holders hear it at their next turn,
	// with where the card went.
	note := fmt.Sprintf("Shared to %s as %s (visible to: %s). Work it there; this private card is closed.", p.BoardName, shared.ShortRef(), strings.Join(p.VisibleTo, ", "))
	if _, err := c.coord.SetTaskState(ctx, c.UserSender().Key, taskID, coord.Canceled, note); err != nil {
		return shared, err
	}
	return shared, nil
}

// WithdrawCard takes a shared card back off its board as the person
// (§5.3): it is canceled with withdrawn set, other members hide it, and
// its holders on this device carry on in a new private card, which is
// returned (zero when nobody here held it).
func (c *Core) WithdrawCard(ctx context.Context, taskID string) (coord.Task, error) {
	boardID, co, err := c.Route(taskID)
	if err != nil {
		return coord.Task{}, err
	}
	if boardID == "" {
		return coord.Task{}, fmt.Errorf("%w: %s is a private card; cancel it instead", coord.ErrInvalid, taskID)
	}
	me := store.PersonActorPrefix + c.chain.PersonID()
	t, err := co.Withdraw(ctx, me, taskID, "withdrawn from the board")
	if err != nil {
		return coord.Task{}, err
	}
	var locals []string
	for _, h := range t.Holders {
		if k := c.PrivateKey(h); k != h && k != c.UserSender().Key {
			locals = append(locals, k)
		}
	}
	// A private card it was shared from comes back into play; otherwise a
	// new private card carries the work on, if anyone here held it.
	for priv, sc := range c.copiesSnapshot() {
		if sc.TaskID == taskID {
			back, n, err := c.coord.Reopen(ctx, c.UserSender().Key, priv)
			if err != nil {
				return coord.Task{}, err
			}
			c.tellNotices(ctx, n)
			for _, k := range locals {
				if _, n, err := c.coord.Assign(ctx, c.UserSender().Key, priv, k); err == nil {
					c.tellNotices(ctx, n)
				}
			}
			return back, nil
		}
	}
	if len(locals) == 0 {
		return coord.Task{}, nil
	}
	steps := make([]coord.Step, 0, len(t.Steps))
	for _, s := range t.Steps {
		steps = append(steps, coord.Step{Text: s.Text, Done: s.Done})
	}
	links := make([]string, 0, len(t.Links))
	for _, l := range t.Links {
		links = append(links, l.Ref)
	}
	nt, err := c.coord.CreateTask(ctx, c.UserSender().Key, coord.NewTask{Title: t.Title, Objective: t.Objective,
		DoneCriteria: t.DoneCriteria, RepoKey: t.RepoKey, Links: links, Steps: steps})
	if err != nil {
		return coord.Task{}, err
	}
	for _, k := range locals {
		_, n, err := c.coord.Assign(ctx, c.UserSender().Key, nt.ID, k)
		if err != nil {
			log.Printf("withdraw %s: assign %s: %v", taskID, k, err)
			continue
		}
		c.tellNotices(ctx, n)
	}
	return nt, nil
}

// tellNotices routes a coord notice from the person at its next turn.
func (c *Core) tellNotices(ctx context.Context, n coord.Notice) {
	if err := c.RouteNotices(ctx, c.UserSender(), n); err != nil && ctx.Err() == nil {
		log.Printf("boards: notice: %v", err)
	}
}

func (c *Core) copiesSnapshot() map[string]sharedCopy {
	c.boards.mu.Lock()
	defer c.boards.mu.Unlock()
	out := make(map[string]sharedCopy, len(c.boards.copies))
	for k, v := range c.boards.copies {
		out[k] = v
	}
	return out
}

package mesh

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"errors"
	"io"
	"sort"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/store"
)

// Board logs between the person's own devices (design v0.5-shared-boards.md
// §12 step 4). Until step 5 gives each board its own topic and connections
// (§6.3), a board log syncs among one person's devices on the private mesh
// connection they already hold, but never on its sync channel: each pull is
// its own stream, labelled board/<boardId> (the label §4.3 reserved), so the
// private rows and rows frames never carry a board row and a board stream
// never carries a private one. Only this person's certified devices are
// served, and only boards both devices hold.
//
// A pull is one stream. The puller writes a 4-byte big-endian length and
// the JSON request {vector}: its board log's vector. The server answers with
// length-prefixed JSON chunks {events} of what the puller lacks (Missing, in
// (device, seq) order), at most boardPullRows rows in all, and closes the
// stream. The puller inserts each device's run; a run refused for "no cert
// yet" is tried again once the rest are in, since a device's cert sits in
// another device's stream (§6.4). Each device pulls from each peer on hello,
// on every vector tick and when a board log here gains a row (KickBoards),
// so both directions run.
const (
	BoardLabelPrefix = "board/"
	maxBoardRequest  = 1 << 20
	boardPullRows    = 4 * MaxBatchRows
)

// BoardLogs are the board logs a Node syncs (Options.Boards). core.Core
// implements it.
type BoardLogs interface {
	// BoardIDs lists the boards open on this device.
	BoardIDs() []string
	// BoardStore is the open log of board id, or nil.
	BoardStore(id string) *store.Store
}

type boardRequest struct {
	Vector store.Vector `json:"vector"`
}

type boardChunk struct {
	Events []Row `json:"events"`
}

// KickBoards pulls every board from every connected device now: a board log
// here gained a row, or a board was opened.
func (n *Node) KickBoards() {
	for _, s := range n.sessionList() {
		select {
		case s.boardWake <- struct{}{}:
		default:
		}
	}
}

// boardLoop pulls each board this device holds from the session's peer,
// once its hello is in and then on every tick and kick, until ctx ends.
func (s *session) boardLoop(ctx context.Context) {
	if s.n.o.Boards == nil {
		return
	}
	tick := time.NewTicker(s.n.iv.Vector)
	defer tick.Stop()
	hello := time.NewTicker(20 * time.Millisecond)
	defer hello.Stop()
	for !s.helloDone() {
		select {
		case <-ctx.Done():
			return
		case <-hello.C:
		}
	}
	hello.Stop()
	for {
		for _, id := range s.n.o.Boards.BoardIDs() {
			if ctx.Err() != nil {
				return
			}
			if err := s.pullBoard(ctx, id); err != nil && ctx.Err() == nil {
				s.n.logf("mesh: board %s from %s: %v", id, s.conn.ID(), err)
			}
		}
		select {
		case <-ctx.Done():
			return
		case <-tick.C:
		case <-s.boardWake:
		}
	}
}

// pullBoard asks the peer for the rows of board id this device lacks and
// inserts them.
func (s *session) pullBoard(ctx context.Context, id string) error {
	st := s.n.o.Boards.BoardStore(id)
	if st == nil {
		return nil
	}
	vec, err := st.Vector(ctx)
	if err != nil {
		return err
	}
	stream, err := s.conn.OpenStream(ctx, BoardLabelPrefix+id)
	if err != nil {
		return err
	}
	stop := context.AfterFunc(ctx, func() { stream.Close() })
	defer stop()
	defer stream.Close()
	if err := writeChunk(stream, boardRequest{Vector: vec}); err != nil {
		return err
	}
	var evs []store.Event
	for {
		var c boardChunk
		if err := readChunk(stream, MaxMessage, &c); err != nil {
			if errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) {
				break
			}
			return err
		}
		for _, r := range c.Events {
			evs = append(evs, r.Event())
		}
		if len(evs) > boardPullRows {
			break // a peer that sends more than asked: what came is enough
		}
	}
	return insertBoardRows(ctx, st, evs)
}

// insertBoardRows inserts evs device by device, trying again the runs
// refused for "no cert yet" while another run got in (§6.4).
func insertBoardRows(ctx context.Context, st *store.Store, evs []store.Event) error {
	runs := map[string][]store.Event{}
	var order []string
	for _, ev := range evs {
		if _, ok := runs[ev.DeviceID]; !ok {
			order = append(order, ev.DeviceID)
		}
		runs[ev.DeviceID] = append(runs[ev.DeviceID], ev)
	}
	var last error
	for len(order) > 0 {
		var wait []string
		moved := false
		for _, dev := range order {
			run := runs[dev]
			sort.Slice(run, func(i, j int) bool { return run[i].Seq < run[j].Seq })
			applied, err := st.InsertBatch(ctx, run)
			if applied > 0 {
				moved = true
			}
			switch {
			case err == nil:
			case errors.Is(err, store.ErrNoCert):
				runs[dev] = run[applied:]
				wait = append(wait, dev)
			case errors.Is(err, store.ErrClosed), errors.Is(err, context.Canceled):
				return nil
			default:
				// A fork, an HLC refusal, a row ahead of the clock: the store
				// recorded what it must; the next pull starts from its vector.
				last = err
			}
		}
		if !moved {
			break
		}
		order = wait
	}
	return last
}

// serveBoard answers one board stream from peer: only a certified device
// of this person, and only a board this device holds.
func (n *Node) serveBoard(peer, label string, st io.ReadWriteCloser) {
	id := strings.TrimPrefix(label, BoardLabelPrefix)
	if n.o.Boards == nil || !n.certified(peer) {
		return
	}
	log := n.o.Boards.BoardStore(id)
	if log == nil {
		return
	}
	var req boardRequest
	if readChunk(st, maxBoardRequest, &req) != nil {
		return
	}
	if req.Vector == nil {
		req.Vector = store.Vector{}
	}
	ctx := context.Background()
	sent := 0
	for sent < boardPullRows {
		evs, err := log.Missing(ctx, req.Vector, MaxBatchRows)
		if err != nil || len(evs) == 0 {
			return
		}
		for _, f := range batches(evs, false) {
			if writeChunk(st, boardChunk{Events: f.Events}) != nil {
				return
			}
		}
		for _, ev := range evs {
			req.Vector[ev.DeviceID] = max(req.Vector[ev.DeviceID], ev.Seq)
		}
		sent += len(evs)
	}
}

func writeChunk(w io.Writer, v any) error {
	body, err := json.Marshal(v)
	if err != nil {
		return err
	}
	_, err = w.Write(append(binary.BigEndian.AppendUint32(nil, uint32(len(body))), body...))
	return err
}

func readChunk(r io.Reader, limit int, v any) error {
	var hdr [4]byte
	if _, err := io.ReadFull(r, hdr[:]); err != nil {
		return err
	}
	size := binary.BigEndian.Uint32(hdr[:])
	if size == 0 || int64(size) > int64(limit) {
		return errors.New("mesh: board chunk too large")
	}
	body := make([]byte, size)
	if _, err := io.ReadFull(r, body); err != nil {
		return err
	}
	return json.Unmarshal(body, v)
}

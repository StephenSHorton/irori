package mesh

import (
	"context"
	"crypto/rand"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strings"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/store"
)

// Blob streams (design §4.3, §6.6): each fetch is its own stream, labelled
// blob/<reqId>. The fetching Core writes a 4-byte big-endian length and the
// JSON request {id, sha256, offset, len} (at most 4 KiB), then reads raw
// bytes until the serving Core closes the stream. The serving Core sends
// only hashes its log references (an attach.offered row) and that it holds;
// for anything else it closes the stream with nothing written.
const (
	BlobLabelPrefix = "blob/"
	maxBlobRequest  = 4 << 10
	blobChunk       = 16 << 10
	// fetchTries is how many times one holder is asked before the next:
	// a stream cut mid-file resumes by offset.
	fetchTries = 3
)

// FileOffer is an attach.offered row's payload.
type FileOffer struct {
	SHA256 string `json:"sha256"`
	Size   int64  `json:"size"`
	Name   string `json:"name,omitempty"`
	Mime   string `json:"mime,omitempty"`
}

// blobRequest is a blob stream's request.
type blobRequest struct {
	ID     string `json:"id"`
	SHA256 string `json:"sha256"`
	Offset int64  `json:"offset"`
	Len    int64  `json:"len"`
}

// OfflineError is FetchFile's answer when no device that holds the file is
// connected. Holders are their device ids, the origin first.
type OfflineError struct{ Holders []string }

func (e *OfflineError) Error() string {
	return "mesh: no device that holds the file is connected (" + strings.Join(e.Holders, ", ") + ")"
}

// ErrNoFiles: the Node was built without an attachment store.
var ErrNoFiles = errors.New("mesh: this device keeps no attachments")

// Offer records that this device holds ref's file and serves it (an
// attach.offered row). An offer this device already wrote is not repeated.
func (n *Node) Offer(ctx context.Context, ref blob.FileRef) error {
	holders, err := n.holders(ctx, ref.SHA256)
	if err != nil {
		return err
	}
	for _, h := range holders {
		if h == n.self {
			return nil
		}
	}
	_, err = n.o.Store.Append(ctx, "irori", store.KindAttachOffered,
		FileOffer{SHA256: ref.SHA256, Size: ref.Size, Name: ref.Name, Mime: ref.Mime})
	return err
}

// holders lists every device whose attach.offered row names sha, in log
// order.
func (n *Node) holders(ctx context.Context, sha string) ([]string, error) {
	var out []string
	seen := map[string]bool{}
	f := store.Filter{Kinds: []string{store.KindAttachOffered}, Limit: store.MaxListLimit}
	for {
		evs, err := n.o.Store.List(ctx, f)
		if err != nil {
			return nil, err
		}
		for _, ev := range evs {
			var o FileOffer
			if json.Unmarshal(ev.Payload, &o) == nil && o.SHA256 == sha && !seen[ev.DeviceID] {
				seen[ev.DeviceID] = true
				out = append(out, ev.DeviceID)
			}
		}
		if len(evs) < f.Limit {
			return out, nil
		}
		f.After = evs[len(evs)-1].Cursor()
	}
}

// FetchFile makes sure ref's file is in this device's attachment store,
// fetching it on demand over a blob stream from the device that attached it
// or from any other device that holds it (§6.6). It returns the device it
// came from ("" when it was already here). A file larger than the cap is
// refused before anything is asked for, and the bytes are checked against
// the ref's length and hash before they are kept. Once stored, this device
// offers it too.
func (n *Node) FetchFile(ctx context.Context, ref blob.FileRef) (string, error) {
	fs := n.o.Files
	if fs == nil {
		return "", ErrNoFiles
	}
	if !blob.ValidID(ref.SHA256) {
		return "", blob.ErrBadID
	}
	if ref.Size > fs.Max() || ref.Size < 0 {
		return "", fmt.Errorf("%w (%d bytes, at most %d)", blob.ErrTooLarge, ref.Size, fs.Max())
	}
	unlock, err := n.lockFetch(ctx, ref.SHA256)
	if err != nil {
		return "", err
	}
	defer unlock()
	if fs.Has(ref.SHA256) {
		return "", nil
	}
	held, err := n.holders(ctx, ref.SHA256)
	if err != nil {
		return "", err
	}
	// The origin first, then every other holder in log order.
	var cands []string
	if ref.Device != "" && ref.Device != n.self {
		cands = append(cands, ref.Device)
	}
	for _, h := range held {
		if h != n.self && h != ref.Device {
			cands = append(cands, h)
		}
	}
	if len(cands) == 0 {
		return "", &OfflineError{}
	}
	var last error
	for _, dev := range cands {
		conn := n.connTo(dev)
		if conn == nil {
			continue
		}
		for try := 0; try < fetchTries; try++ {
			err := n.fetchFrom(ctx, conn, ref)
			if err == nil {
				if err := n.Offer(ctx, ref); err != nil {
					n.logf("mesh: offer %s: %v", ref.SHA256, err)
				}
				return dev, nil
			}
			last = err
			if ctx.Err() != nil {
				return "", ctx.Err()
			}
			if !errors.Is(err, blob.ErrMismatch) || errors.Is(err, errNotServed) {
				break
			}
		}
	}
	if last != nil {
		return "", last
	}
	return "", &OfflineError{Holders: cands}
}

var errNotServed = fmt.Errorf("%w: the device doesn't serve it", blob.ErrMismatch)

// fetchFrom asks conn for the rest of ref's file and moves it into place
// once it is whole.
func (n *Node) fetchFrom(ctx context.Context, conn Conn, ref blob.FileRef) error {
	fs := n.o.Files
	part, off, err := fs.Partial(ref.SHA256)
	if err != nil {
		return err
	}
	if off > ref.Size {
		part.Close()
		fs.DropPartial(ref.SHA256)
		return blob.ErrMismatch
	}
	var got int64
	if off < ref.Size {
		got, err = n.readBlob(ctx, conn, ref, off, part)
	}
	if cerr := part.Close(); err == nil {
		err = cerr
	}
	if err != nil {
		return err
	}
	if off < ref.Size && got == 0 {
		return errNotServed
	}
	return fs.Finish(ref.SHA256, ref.Size)
}

// readBlob opens one blob stream and appends what it carries to w, never
// more than the ref still lacks (a longer stream is a mismatch).
func (n *Node) readBlob(ctx context.Context, conn Conn, ref blob.FileRef, off int64, w io.Writer) (int64, error) {
	id := make([]byte, 8)
	rand.Read(id)
	req := blobRequest{ID: hex.EncodeToString(id), SHA256: ref.SHA256, Offset: off, Len: ref.Size - off}
	st, err := conn.OpenStream(ctx, BlobLabelPrefix+req.ID)
	if err != nil {
		return 0, err
	}
	// A cancelled fetch closes its stream, which also unblocks the server.
	stop := context.AfterFunc(ctx, func() { st.Close() })
	defer stop()
	defer st.Close()
	body, _ := json.Marshal(req)
	msg := binary.BigEndian.AppendUint32(nil, uint32(len(body)))
	if _, err := st.Write(append(msg, body...)); err != nil {
		return 0, err
	}
	want := req.Len
	got, err := io.CopyBuffer(w, io.LimitReader(st, want+1), make([]byte, blobChunk))
	if err != nil && ctx.Err() != nil {
		return got, ctx.Err()
	}
	if got > want {
		n.o.Files.DropPartial(ref.SHA256)
		return got, blob.ErrMismatch
	}
	// An error after some bytes is a cut stream: the caller resumes.
	if err != nil && got == 0 {
		return 0, err
	}
	return got, nil
}

// serveStream answers one stream the peer opened. Only certified,
// unrevoked devices are served, and only blob streams.
func (n *Node) serveStream(peer, label string, st io.ReadWriteCloser) {
	defer st.Close()
	if !strings.HasPrefix(label, BlobLabelPrefix) || n.o.Files == nil || !n.certified(peer) {
		return
	}
	var hdr [4]byte
	if _, err := io.ReadFull(st, hdr[:]); err != nil {
		return
	}
	size := binary.BigEndian.Uint32(hdr[:])
	if size == 0 || size > maxBlobRequest {
		return
	}
	body := make([]byte, size)
	if _, err := io.ReadFull(st, body); err != nil {
		return
	}
	var req blobRequest
	if json.Unmarshal(body, &req) != nil || !blob.ValidID(req.SHA256) || req.Offset < 0 || req.Len < 0 {
		return
	}
	// Only hashes the log references, and that are here.
	held, err := n.holders(context.Background(), req.SHA256)
	if err != nil || len(held) == 0 || !n.o.Files.Has(req.SHA256) {
		return
	}
	f, err := n.o.Files.Open(req.SHA256)
	if err != nil {
		return
	}
	defer f.Close()
	fi, err := f.Stat()
	if err != nil || fi.Size() > n.o.Files.Max() || req.Offset > fi.Size() {
		return
	}
	if _, err := f.Seek(req.Offset, io.SeekStart); err != nil {
		return
	}
	left := fi.Size() - req.Offset
	if req.Len > 0 {
		left = min(left, req.Len)
	}
	if _, err := io.CopyBuffer(st, io.LimitReader(f, left), make([]byte, blobChunk)); err != nil {
		n.logf("mesh: serve %s to %s: %v", req.SHA256, peer, err)
	}
}

// connTo is a live, synced connection to dev, or nil.
func (n *Node) connTo(dev string) Conn {
	for _, s := range n.sessionList() {
		if s.conn.ID() == dev && s.helloDone() {
			select {
			case <-s.conn.Done():
				continue
			default:
			}
			return s.conn
		}
	}
	return nil
}

// lockFetch waits its turn to fetch sha (one fetch per attachment at a
// time), or gives up with ctx's error, so a stalled fetch never holds a
// caller past its own deadline. unlock gives the turn back.
func (n *Node) lockFetch(ctx context.Context, sha string) (unlock func(), err error) {
	n.mu.Lock()
	if n.fetching == nil {
		n.fetching = map[string]chan struct{}{}
	}
	turn := n.fetching[sha]
	if turn == nil {
		turn = make(chan struct{}, 1)
		n.fetching[sha] = turn
	}
	n.mu.Unlock()
	select {
	case turn <- struct{}{}:
		return func() { <-turn }, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

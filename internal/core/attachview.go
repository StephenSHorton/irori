package core

import (
	"context"
	"errors"
	"fmt"
	"io"
	"mime"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/mesh"
)

// The window's view of Board attachments (card #14): the GUI is the
// person, who may read every comment's files. Only Board comments count
// here, never messages, so a hash off a message can't be read through the
// window. AttachmentHandler is served by the app's own webview (a Wails
// asset-server route), never on the loopback API.

// AttachmentRoute is the path prefix the window loads a comment's file
// from: AttachmentRoute + "<sha256>".
const AttachmentRoute = "/irori/attachment/"

// InlineImages are the types the route serves for the window to show, by
// the file's sniffed bytes (never its name). Everything else is a
// download.
var InlineImages = []string{"image/png", "image/jpeg", "image/gif", "image/webp"}

// FileOfflineError: the file isn't on this device and no device that
// holds it is connected. On names those devices.
type FileOfflineError struct{ On string }

func (e *FileOfflineError) Error() string {
	return fmt.Sprintf("the file is on %s, which isn't connected now; try again when it is", e.On)
}

// fileOffline turns mesh's OfflineError into a FileOfflineError naming
// the holders (the attaching device when no holder is known); other
// errors pass through.
func (c *Core) fileOffline(err error, ref blob.FileRef) error {
	var off *mesh.OfflineError
	if !errors.As(err, &off) {
		return err
	}
	ids := off.Holders
	if len(ids) == 0 && ref.Device != "" && ref.Device != c.device.ID {
		ids = []string{ref.Device}
	}
	return &FileOfflineError{On: c.deviceNames(ids)}
}

// CommentFile finds a file attached to a Board comment (ErrBadID for a
// malformed hash, ErrNoAttachment when no comment carries it).
func (c *Core) CommentFile(sha string) (blob.FileRef, error) {
	sha = strings.ToLower(strings.TrimSpace(sha))
	if !blob.ValidID(sha) {
		return blob.FileRef{}, blob.ErrBadID
	}
	ref, ok := c.coord.CommentAttachment(sha)
	if !ok {
		return blob.FileRef{}, ErrNoAttachment
	}
	ref.SHA256 = sha
	return ref, nil
}

// HasFile reports whether an attachment is already on this device.
func (c *Core) HasFile(sha string) bool { return c.files.Has(sha) }

// DeviceName is a device's label, else its id.
func (c *Core) DeviceName(id string) string {
	if id == "" {
		return ""
	}
	return c.deviceNames([]string{id})
}

// FetchCommentFile makes sure a comment's file is on this device, fetching
// it from a device that holds it first. from is the device it came from
// ("" when it was already here). A FileOfflineError when no holder is
// connected.
func (c *Core) FetchCommentFile(ctx context.Context, sha string) (ref blob.FileRef, from string, err error) {
	if ref, err = c.CommentFile(sha); err != nil {
		return blob.FileRef{}, "", err
	}
	if from, err = c.mesh.FetchFile(ctx, ref); err != nil {
		return blob.FileRef{}, "", c.fileOffline(err, ref)
	}
	return ref, from, nil
}

// OpenCommentFile is FetchCommentFile, then the stored file opened for
// reading. The caller closes it.
func (c *Core) OpenCommentFile(ctx context.Context, sha string) (blob.FileRef, *os.File, error) {
	ref, _, err := c.FetchCommentFile(ctx, sha)
	if err != nil {
		return blob.FileRef{}, nil, err
	}
	f, err := c.files.Open(ref.SHA256)
	if err != nil {
		return blob.FileRef{}, nil, err
	}
	return ref, f, nil
}

// SaveCommentFile writes a comment's file into dir under its safe name,
// fetching it first if it isn't here. An existing file is never
// overwritten: the name gets " (2)", " (3)" ... unless that file already
// holds these bytes, which is then the answer. dir must be absolute and
// outside the data dir.
func (c *Core) SaveCommentFile(ctx context.Context, sha, dir string) (string, error) {
	if !filepath.IsAbs(dir) {
		return "", fmt.Errorf("%q is not an absolute folder", dir)
	}
	data, err := filepath.Abs(c.opts.DataDir)
	if err != nil {
		return "", err
	}
	realData := data
	if real, err := filepath.EvalSymlinks(data); err == nil {
		realData = real
	}
	if dir = filepath.Clean(dir); blob.Within(dir, data) || blob.Within(resolveExisting(dir), realData) {
		return "", fmt.Errorf("%s is inside Irori's data dir", dir)
	}
	ref, _, err := c.FetchCommentFile(ctx, sha)
	if err != nil {
		return "", err
	}
	dst := freeName(filepath.Join(dir, blob.SafeName(ref.Name, ref.SHA256)), ref.SHA256)
	if blob.SameContent(dst, ref.SHA256) {
		return dst, nil
	}
	if err := c.files.CopyOut(ref.SHA256, dst); err != nil {
		if errors.Is(err, os.ErrExist) {
			// Taken between the look and the write: one more free name.
			dst = freeName(dst, ref.SHA256)
			err = c.files.CopyOut(ref.SHA256, dst)
		}
		if err != nil {
			return "", err
		}
	}
	return dst, nil
}

// SniffFile is a stored file's type by its first 512 bytes.
func SniffFile(f io.ReadSeeker) (string, error) {
	head := make([]byte, 512)
	n, err := io.ReadFull(f, head)
	if err != nil && !errors.Is(err, io.ErrUnexpectedEOF) && !errors.Is(err, io.EOF) {
		return "", err
	}
	if _, err := f.Seek(0, io.SeekStart); err != nil {
		return "", err
	}
	t, _, _ := strings.Cut(http.DetectContentType(head[:n]), ";")
	return t, nil
}

// InlineImage reports whether the window may show a sniffed type inline.
func InlineImage(sniffed string) bool {
	for _, t := range InlineImages {
		if sniffed == t {
			return true
		}
	}
	return false
}

// AttachmentHandler serves AttachmentRoute + "<sha256>" (GET and HEAD) for
// the window: the file of a Board comment, fetched from another device
// first when it isn't here. A PNG, JPEG, GIF or WebP (by its bytes) is
// served inline with that type; anything else as an
// application/octet-stream download. Every answer is nosniff and
// sandboxed. 404 for a hash no comment carries, 503 with the reason as
// plain text when the devices that hold it are offline.
func (c *Core) AttachmentHandler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		h := w.Header()
		h.Set("X-Content-Type-Options", "nosniff")
		h.Set("Content-Security-Policy", "default-src 'none'; sandbox")
		if r.Method != http.MethodGet && r.Method != http.MethodHead {
			h.Set("Allow", "GET, HEAD")
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		sha, ok := strings.CutPrefix(r.URL.Path, AttachmentRoute)
		if !ok || strings.Contains(sha, "/") {
			http.NotFound(w, r)
			return
		}
		ref, f, err := c.OpenCommentFile(r.Context(), sha)
		if err != nil {
			h.Set("Cache-Control", "no-store")
			var off *FileOfflineError
			switch {
			case errors.Is(err, blob.ErrBadID), errors.Is(err, ErrNoAttachment):
				http.Error(w, ErrNoAttachment.Error(), http.StatusNotFound)
			case errors.As(err, &off):
				http.Error(w, off.Error(), http.StatusServiceUnavailable)
			case errors.Is(err, blob.ErrTooLarge):
				http.Error(w, err.Error(), http.StatusRequestEntityTooLarge)
			default:
				http.Error(w, err.Error(), http.StatusBadGateway)
			}
			return
		}
		defer f.Close()
		typ, err := SniffFile(f)
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		name := blob.SafeName(ref.Name, ref.SHA256)
		disp := "attachment"
		if InlineImage(typ) {
			disp = "inline"
		} else {
			typ = "application/octet-stream"
		}
		h.Set("Content-Type", typ)
		if v := mime.FormatMediaType(disp, map[string]string{"filename": name}); v != "" {
			disp = v
		}
		h.Set("Content-Disposition", disp)
		// The bytes are named by their hash, so they never change.
		h.Set("Cache-Control", "private, max-age=31536000, immutable")
		h.Set("ETag", `"`+ref.SHA256+`"`)
		http.ServeContent(w, r, "", time.Time{}, f)
	})
}

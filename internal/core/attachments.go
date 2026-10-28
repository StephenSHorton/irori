package core

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/mesh"
)

// ErrNoAttachment: no message or card the caller may read carries the file.
var ErrNoAttachment = errors.New("no message or card you can read carries that attachment")

// AttachFiles checks and stores files an agent attaches to a message or a
// comment (v0.4 §6.6): each path resolves to its canonical form, following
// symlinks, and anything the never-leave rules cover is refused with the
// reason; the file is copied into the attachment store and offered to the
// person's other devices. A relative path is taken against the caller's
// working directory. Nothing inside a file is scrubbed: it is the person's.
func (c *Core) AttachFiles(ctx context.Context, by string, paths []string) ([]blob.FileRef, error) {
	if len(paths) == 0 {
		return nil, nil
	}
	if len(paths) > inbox.MaxAttachments {
		return nil, fmt.Errorf("at most %d attachments", inbox.MaxAttachments)
	}
	cwd := ""
	for _, a := range c.LocalAgents() {
		if a.Key == by {
			cwd = a.Cwd
			break
		}
	}
	out := make([]blob.FileRef, 0, len(paths))
	for _, p := range paths {
		real, err := blob.CheckAttachPath(p, cwd, c.opts.DataDir)
		if err != nil {
			return nil, err
		}
		ref, err := c.files.PutFile(real)
		if err != nil {
			return nil, fmt.Errorf("%s: %w", filepath.Base(real), err)
		}
		// The name the agent gave, reduced to one safe file name.
		ref.Name = blob.SafeName(filepath.Base(strings.TrimSpace(p)), ref.SHA256)
		ref.Device = c.device.ID
		if err := c.mesh.Offer(ctx, ref); err != nil {
			return nil, err
		}
		out = append(out, ref)
	}
	return out, nil
}

// GetAttachment fetches a file reader may read (on a message it sent or
// received, or on any Board comment) and writes it to to, or by default
// to <dataDir>/received/<session>/<name>. It returns the local path and the
// device it came from ("" when it was already here). A to inside the data
// dir is refused, and an existing file is never overwritten.
func (c *Core) GetAttachment(ctx context.Context, reader, sha, to string) (path, from string, err error) {
	sha = strings.ToLower(strings.TrimSpace(sha))
	if !blob.ValidID(sha) {
		return "", "", blob.ErrBadID
	}
	ref, ok := c.inbox.Attachment(reader, sha)
	if !ok {
		ref, ok = c.coord.CommentAttachment(sha)
	}
	if !ok {
		return "", "", ErrNoAttachment
	}
	name := blob.SafeName(ref.Name, sha)
	dst, err := c.attachmentDst(reader, name, to)
	if err != nil {
		return "", "", err
	}
	from, err = c.mesh.FetchFile(ctx, ref)
	if err != nil {
		var off *mesh.OfflineError
		if errors.As(err, &off) {
			return "", "", fmt.Errorf("the file is on %s, which isn't connected now; try again when it is", c.deviceNames(off.Holders))
		}
		return "", "", err
	}
	if to == "" {
		dst = freeName(dst, sha)
		if blob.SameContent(dst, sha) {
			return dst, from, nil
		}
	} else if _, err := os.Lstat(dst); err == nil {
		if blob.SameContent(dst, sha) {
			return dst, from, nil
		}
		return "", "", fmt.Errorf("%s already exists; give another to", dst)
	}
	if err := c.files.CopyOut(sha, dst); err != nil {
		return "", "", err
	}
	return dst, from, nil
}

// attachmentDst is where a fetched file goes: to (a file, or an existing
// folder to put name in), never inside the data dir, or the caller's own
// folder under ReceivedDir.
func (c *Core) attachmentDst(reader, name, to string) (string, error) {
	data, err := filepath.Abs(c.opts.DataDir)
	if err != nil {
		return "", err
	}
	if to = strings.TrimSpace(to); to == "" {
		folder := blob.SafeName(strings.ReplaceAll(reader, "/", "_"), "")
		if folder == "" || strings.HasPrefix(folder, "attachment-") {
			folder = "unknown"
		}
		return filepath.Join(data, blob.ReceivedDir, folder, name), nil
	}
	if !filepath.IsAbs(to) {
		cwd := ""
		for _, a := range c.LocalAgents() {
			if a.Key == reader {
				cwd = a.Cwd
			}
		}
		if cwd == "" || !filepath.IsAbs(cwd) {
			return "", fmt.Errorf("%q is relative and your working directory is unknown: give an absolute path", to)
		}
		to = filepath.Join(cwd, to)
	}
	if fi, err := os.Stat(to); err == nil && fi.IsDir() {
		to = filepath.Join(to, name)
	}
	to = filepath.Clean(to)
	// Resolve what exists of it, so a link can't point the write into the
	// data dir.
	parent := filepath.Dir(to)
	if real, err := filepath.EvalSymlinks(parent); err == nil {
		parent = real
	}
	realData := data
	if real, err := filepath.EvalSymlinks(data); err == nil {
		realData = real
	}
	if blob.Within(parent, realData) || blob.Within(to, data) {
		return "", fmt.Errorf("%s is inside Irori's data dir; leave to empty for the default folder", to)
	}
	return to, nil
}

// freeName is dst, or dst with " (2)", " (3)" ... before its extension when
// another file already holds that name.
func freeName(dst, sha string) string {
	if _, err := os.Lstat(dst); err != nil || blob.SameContent(dst, sha) {
		return dst
	}
	ext := filepath.Ext(dst)
	base := strings.TrimSuffix(dst, ext)
	for i := 2; ; i++ {
		p := fmt.Sprintf("%s (%d)%s", base, i, ext)
		if _, err := os.Lstat(p); err != nil || blob.SameContent(p, sha) {
			return p
		}
	}
}

// deviceNames names devices by their cert labels, for a message.
func (c *Core) deviceNames(ids []string) string {
	if len(ids) == 0 {
		return "no device that is connected"
	}
	var names []string
	for _, id := range ids {
		name := id
		if cert, ok := c.chain.Cert(id); ok && cert.Label != "" {
			name = cert.Label
		}
		names = append(names, name)
	}
	return strings.Join(names, " or ")
}

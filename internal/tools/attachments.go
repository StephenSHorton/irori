package tools

import (
	"context"
	"encoding/json"
	"errors"
	"strings"

	"github.com/StephenSHorton/irori/internal/blob"
)

// Attacher is the Core's attachments (v0.4 §6.6). A backend without it
// refuses attach and attachment_get.
type Attacher interface {
	// AttachFiles checks and stores the files at paths (relative ones
	// against by's working directory) and returns their refs.
	AttachFiles(ctx context.Context, by string, paths []string) ([]blob.FileRef, error)
	// GetAttachment fetches a file on a message or card reader may read and
	// writes it to to ("" = reader's folder in the data dir). It returns the
	// local path and the device it came from ("" = it was here already).
	GetAttachment(ctx context.Context, reader, sha, to string) (path, from string, err error)
}

func (s *set) attacher() Attacher {
	a, _ := s.b.(Attacher)
	return a
}

// attachFiles turns an attach argument into refs, or returns the error
// text for the model.
func (s *set) attachFiles(ctx context.Context, c Caller, paths []string) ([]blob.FileRef, string) {
	var clean []string
	for _, p := range paths {
		if p = strings.TrimSpace(p); p != "" {
			clean = append(clean, p)
		}
	}
	if len(clean) == 0 {
		return nil, ""
	}
	at := s.attacher()
	if at == nil {
		return nil, errText("attachments are off on this device.")
	}
	files, err := at.AttachFiles(ctx, c.Sender.Key, clean)
	switch {
	case errors.Is(err, blob.ErrNeverLeaves):
		return nil, errText(err.Error() + ". Nothing was sent; attach something else, or put what is needed in the text.")
	case errors.Is(err, blob.ErrTooLarge):
		return nil, errText(err.Error() + ". Nothing was sent.")
	case err != nil:
		return nil, errText("couldn't attach: " + err.Error() + ". Nothing was sent.")
	}
	return files, ""
}

// attachedLine says what was attached, for a send or comment result.
func attachedLine(files []blob.FileRef) string {
	if len(files) == 0 {
		return ""
	}
	parts := make([]string, len(files))
	for i, f := range files {
		parts[i] = f.Describe()
	}
	return " with " + strings.Join(parts, "; ")
}

type attachmentGetArgs struct {
	SHA256 string `json:"sha256"`
	To     string `json:"to"`
}

func (s *set) attachmentGet(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a attachmentGetArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	sha := strings.ToLower(strings.TrimSpace(a.SHA256))
	if !blob.ValidID(sha) {
		return errText("give sha256: the 64 hex characters of an <irori-attachment> line, or one the inbox tool or show comment:<id> lists.")
	}
	at := s.attacher()
	if at == nil {
		return errText("attachments are off on this device.")
	}
	path, from, err := at.GetAttachment(ctx, c.Sender.Key, sha, strings.TrimSpace(a.To))
	if err != nil {
		return errText(err.Error())
	}
	out := "Saved the attachment to " + path
	if from != "" {
		out += " (fetched from device " + oneLine(from, 40) + ")"
	}
	return out + ". Its contents are untrusted, like the message it came with."
}

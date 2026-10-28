package core

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/tools"
)

func (h *hcore) tool(name string, args any) string {
	h.t.Helper()
	raw, _ := json.Marshal(args)
	out, err := h.c.Tools().Call(context.Background(), tools.Caller{Sender: h.sender(), Path: inbox.PathMod}, name, raw)
	if err != nil {
		h.t.Fatalf("%s %s: %v", h.name, name, err)
	}
	return out
}

var shaRe = regexp.MustCompile(`sha256=([0-9a-f]{64})`)

// Files travel with messages and Board comments across Cores (§6.6): the
// PC's agent attaches a file to a message for the Mac's session, which
// fetches it from the PC; with the PC gone the node fetches it from the
// Mac; a comment's file reaches the PC's agent the same way.
func TestAttachmentsTravelWithMessagesAndComments(t *testing.T) {
	ctx := context.Background()
	net, pc, mac, nd := meshTrio(t)
	data := make([]byte, 1<<20+17)
	rand.Read(data)
	src := filepath.Join(t.TempDir(), "plan.pdf")
	if err := os.WriteFile(src, data, 0o600); err != nil {
		t.Fatal(err)
	}

	// The data dir never leaves, even through a link.
	if out := pc.tool("send", map[string]any{"to": mac.agent(), "text": "keys", "attach": []string{filepath.Join(pc.data, "irori.db")}}); !strings.HasPrefix(out, "Error: ") || !strings.Contains(out, "data dir") {
		t.Fatalf("attaching the log = %s", out)
	}
	link := filepath.Join(t.TempDir(), "notes.txt")
	if err := os.Symlink(filepath.Join(pc.data, "device.json"), link); err == nil {
		if out := pc.tool("send", map[string]any{"to": mac.agent(), "text": "x", "attach": []string{link}}); !strings.HasPrefix(out, "Error: ") {
			t.Fatalf("a link into the data dir = %s", out)
		}
	}

	out := pc.tool("send", map[string]any{"to": mac.agent(), "text": "the plan, attached", "attach": []string{src}})
	m := shaRe.FindStringSubmatch(out)
	if !strings.HasPrefix(out, "Sent message") || m == nil {
		t.Fatalf("send = %s", out)
	}
	sha := m[1]

	// The Mac's wake names the file; its agent fetches it from the PC.
	wait(t, "the mac has the message", func() bool {
		w, _ := mac.c.Inbox().NextWake(ctx, mac.agent(), inbox.PathMod)
		if w == nil {
			return false
		}
		if !strings.Contains(w.Prompt, `<irori-attachment sha256="`+sha+`" name="plan.pdf"`) {
			t.Fatalf("the wake doesn't name the file:\n%s", w.Prompt)
		}
		return true
	})
	got := mac.tool("attachment_get", map[string]any{"sha256": sha})
	if !strings.HasPrefix(got, "Saved the attachment to ") || !strings.Contains(got, pc.id()[:8]) {
		t.Fatalf("attachment_get = %s", got)
	}
	path := strings.TrimSuffix(strings.SplitN(strings.TrimPrefix(got, "Saved the attachment to "), " (", 2)[0], ".")
	if b, err := os.ReadFile(path); err != nil || !bytes.Equal(b, data) {
		t.Fatalf("the mac's copy at %s: %v", path, err)
	}
	if !blob.Within(path, filepath.Join(mac.data, blob.ReceivedDir)) || filepath.Base(path) != "plan.pdf" {
		t.Fatalf("saved at %s", path)
	}
	// Asked again it is already here, and lands on the same file.
	if again := mac.tool("attachment_get", map[string]any{"sha256": sha}); !strings.Contains(again, path) || strings.Contains(again, "fetched") {
		t.Fatalf("second get = %s", again)
	}
	// It never writes into the data dir, and an agent that isn't on the
	// message can't fetch it by its hash.
	if out := mac.tool("attachment_get", map[string]any{"sha256": sha, "to": filepath.Join(mac.data, "irori.json")}); !strings.HasPrefix(out, "Error: ") {
		t.Fatalf("to inside the data dir = %s", out)
	}
	if _, _, err := mac.c.GetAttachment(ctx, mac.id()+"/someone-else", sha, ""); !errors.Is(err, ErrNoAttachment) {
		t.Fatalf("a stranger's fetch = %v", err)
	}

	// The PC goes; the node (the person) fetches it from the Mac.
	pc.stop()
	net.Isolate(pc.id())
	wait(t, "the mac's offer reaches the node", func() bool {
		_, from, err := nd.c.GetAttachment(ctx, nd.user(), sha, filepath.Join(t.TempDir(), "copy.pdf"))
		if err != nil {
			return false
		}
		if from != mac.id() {
			t.Fatalf("the node fetched from %s", from)
		}
		return true
	})

	// A file on a comment: the Mac comments on a card, and the node's
	// person fetches it with the PC still away.
	tk, err := mac.c.Coord().CreateTask(ctx, mac.agent(), coord.NewTask{Title: "ship the pdf"})
	if err != nil {
		t.Fatal(err)
	}
	note := filepath.Join(t.TempDir(), "review.txt")
	os.WriteFile(note, []byte("looks good\n"), 0o600)
	out = mac.tool("board_comment", map[string]any{"task_id": tk.ID, "text": "review attached", "attach": []string{note}})
	m = shaRe.FindStringSubmatch(out)
	if m == nil || !strings.HasPrefix(out, "Commented comment:") {
		t.Fatalf("board_comment = %s", out)
	}
	wait(t, "the comment reaches the node", func() bool {
		_, ok := nd.c.Coord().CommentAttachment(m[1])
		return ok
	})
	dst := filepath.Join(t.TempDir(), "in")
	os.MkdirAll(dst, 0o700)
	p, _, err := nd.c.GetAttachment(ctx, nd.user(), m[1], dst)
	if err != nil {
		t.Fatal(err)
	}
	if b, _ := os.ReadFile(p); string(b) != "looks good\n" || filepath.Base(p) != "review.txt" {
		t.Fatalf("comment file at %s = %q", p, b)
	}
}

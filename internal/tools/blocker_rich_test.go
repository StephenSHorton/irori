package tools

import (
	"context"
	"path/filepath"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/blob"
)

// attachingBackend is the harness backend with attachments on: each path
// becomes a ref named after it, its hash made from the name.
type attachingBackend struct{ *backend }

func (attachingBackend) AttachFiles(_ context.Context, _ string, paths []string) ([]blob.FileRef, error) {
	var out []blob.FileRef
	for i, p := range paths {
		out = append(out, blob.FileRef{SHA256: strings.Repeat(string(rune('a'+i)), 64), Name: filepath.Base(p), Size: 10, Device: "dev1"})
	}
	return out, nil
}

func (attachingBackend) GetAttachment(context.Context, string, string, string) (string, string, error) {
	return "", "", nil
}

// board_block carries links and files on the blocker, and a link on each
// option; show and the board list them for agents.
func TestBoardBlockLinksAndAttach(t *testing.T) {
	h := newHarness(t)
	h.set = New(attachingBackend{h.b})
	h.set.impl.now = h.clk.now
	id := h.newCard(callA, map[string]any{"title": "Train the probe"})
	h.ok(callA, "board_join", map[string]any{"task_id": id})

	h.fails(callA, "board_block", map[string]any{"task_id": id, "question": "q?", "links": []string{"ftp://nope"}}, "http(s)")
	h.fails(callA, "board_block", map[string]any{"task_id": id, "question": "q?",
		"options": []map[string]any{{"answer": "a", "link": "data://x"}}}, "http(s)")
	h.fails(callA, "board_block", map[string]any{"task_id": id, "question": "q?",
		"links": []string{"a", "b", "c", "d", "e", "f", "g", "h", "i"}}, "at most 8")

	url := "https://huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m"
	out := h.ok(callA, "board_block", map[string]any{"task_id": id, "question": "Request DINOv3 access?",
		"context": "Open " + url + " and click Request access, then type `! hf auth login` in the Mac session.",
		"links":   []string{url, "docs/models.md"},
		"attach":  []string{"shots/gate.png"},
		"options": []map[string]any{
			{"answer": "I requested it", "recommended": true, "link": "https://huggingface.co/settings/gated-repos"},
			{"answer": "Use DINOv2 instead"},
		}})
	has(t, out, "Blocked #1 on blocker b1 with gate.png", "and links huggingface.co, docs/models.md.")

	tk, _ := h.b.co.GetTask(id)
	b := tk.OpenBlockers()[0]
	if len(b.Links) != 2 || b.Links[0].Ref != url || len(b.Attachments) != 1 || b.Attachments[0].Name != "gate.png" ||
		b.Options[0].Link == nil || b.Options[0].Link.Ref != "https://huggingface.co/settings/gated-repos" || b.Options[1].Link != nil {
		t.Fatalf("blocker %+v", b)
	}
	for _, o := range []string{out, h.ok(callB, "board", nil)} {
		has(t, o, "links: "+url+", docs/models.md", "file: gate.png", "(attachment_get)",
			`"I requested it" (alpha) [recommended] link: https://huggingface.co/settings/gated-repos`)
	}

	// Links alone still raise an item (not the reason-only form).
	id2 := h.newCard(callA, map[string]any{"title": "Second"})
	h.ok(callA, "board_join", map[string]any{"task_id": id2})
	has(t, h.ok(callA, "board_block", map[string]any{"task_id": id2, "reason": "waiting on the vendor", "links": []string{"https://example.com/status"}}),
		"with links example.com")
	if tk, _ := h.b.co.GetTask(id2); len(tk.OpenBlockers()) != 1 || len(tk.OpenBlockers()[0].Links) != 1 {
		t.Fatalf("second %+v", tk.Blockers)
	}
}

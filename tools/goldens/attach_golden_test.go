// Golden exporter for crates/attach (surface.md §2.6, architecture.md §6.2).
//
// Injected into I:internal/api by tools/goldens/export.sh with
// `go test -overlay`, so it runs in package api, beside stream.go's
// Preview; it runs only when IRORI_GOLDEN_OUT is set. It writes
// attach/attach.json:
//
//   - events: json.Marshal(attach.Event) for every event type the Core
//     and the runner write, with every field the type carries, Go's HTML
//     and U+2028 escapes and omitempty zeros (field order, names and
//     escaping are what the Rust Event must reproduce byte for byte);
//   - previews: Preview over whitespace, control and long-text inputs;
//   - modAtLeast: attach.ModAtLeast over a version table, against
//     AppendSince and other mins.
package api

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/attach"
)

func TestGoldenAttach(t *testing.T) {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	prompt := "[irori-wake:wake_01]\nIrori message.\n<irori-message from=\"bob\" key=\"dev1/s2\" delivery=\"del_1\" trust=\"untrusted\">a & b \u2028 c\u2029</irori-message>"
	evs := []attach.Event{
		{Type: attach.TypeHello, V: attach.Version, Session: "b886ef92-0000-4000-8000-000000000001",
			AgentKey: "dev1/b886ef92-0000-4000-8000-000000000001", Name: "api <work>", NameSource: "user", AITitle: "Fix the lexer & parser",
			API: &attach.API{URL: "http://127.0.0.1:47811", Token: "bearer-1"}, AttachSecret: "secret-1"},
		{Type: attach.TypeHello, V: attach.Version, Session: "grok-01ABC", AgentKey: "dev1/grok-01ABC", Name: "grokky",
			API: &attach.API{URL: "http://127.0.0.1:47811", Token: ""}},
		{Type: attach.TypeDeliver, WakeID: "wake_01", Token: "[irori-wake:wake_01]", DeliveryIDs: []string{"del_1", "del_2"},
			MessageIDs: []string{"msg_1", "msg_2"}, Prompt: prompt, From: "bob", Overflow: []string{"del_2"}},
		{Type: attach.TypeDeliver, WakeID: "wake_02", Token: "[irori-wake:wake_02]", Reminder: true, DeliveryIDs: []string{"del_3"},
			Prompt: "[irori-wake:wake_02] reminder", Mode: "next_turn"},
		{Type: attach.TypeAppend, WakeID: "wake_03", Token: "[irori-append:wake_03]", DeliveryIDs: []string{"del_4"},
			MessageIDs: []string{"msg_4"}, Prompt: "<irori-message mode=\"next_turn\">fyi</irori-message>\n[irori-append:wake_03]", From: "you", Mode: "next_turn"},
		{Type: attach.TypeName, Name: "renamed", NameSource: "irori", AITitle: "t"},
		{Type: attach.TypeName},
		{Type: attach.TypeHeld, DeliveryID: "del_5", From: "grok-1", Why: "bypass_receiver", Preview: "x <y>"},
		{Type: attach.TypeMoved, DeliveryID: "del_6", To: "front desk"},
		{Type: attach.TypeInbox, Unread: 3, HeldN: 1},
		{Type: attach.TypeInbox, HeldN: 2},
		{Type: attach.TypeInbox},
		{Type: attach.TypeSuperseded},
		{Type: attach.TypeError, Message: "Irori isn't running, so messages can't reach this session. It attaches by itself once Irori starts."},
	}
	type line struct {
		Line string `json:"line"`
	}
	var events []line
	for _, ev := range evs {
		raw, err := json.Marshal(ev)
		if err != nil {
			t.Fatal(err)
		}
		events = append(events, line{string(raw)})
	}
	type preview struct {
		In  string `json:"in"`
		Out string `json:"out"`
	}
	var previews []preview
	for _, in := range []string{
		"", "plain", "  lead and trail  ", "a\n\tb\r\nc", "nul\x00bel\x07del\x7f", "nbsp\u00a0here\u2028there\u0085x",
		strings.Repeat("\u00e9", 130), strings.Repeat("ab ", 60), "\u200bzero width\ufeff",
	} {
		previews = append(previews, preview{in, Preview(in)})
	}
	type mod struct {
		V    string `json:"v"`
		Min  string `json:"min"`
		Want bool   `json:"want"`
	}
	var mods []mod
	for _, min := range []string{attach.AppendSince, "", "junk", "1.2.3", "0.4"} {
		for _, v := range []string{"", "junk", ".4.0", "0.2.0-test", "0.3.99999", "0.4.0", "0.4", "0.4.0-rc1", "0.4.1",
			"0.10.0", "1.0", "1", "v0.4.0", " 0.4.0 ", "\t0.4.0\n", "vv0.4.0", "0.4-rc1.9", "0-rc.9.9", "1234567.0.0",
			"0.0000004.0", "1.2.3.4", "1.2.3x", "\u0661.\u0662.\u0663"} {
			mods = append(mods, mod{v, min, attach.ModAtLeast(v, min)})
		}
	}
	out := map[string]any{"events": events, "previews": previews, "modAtLeast": mods}
	path := filepath.Join(dir, "attach", "attach.json")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(out); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

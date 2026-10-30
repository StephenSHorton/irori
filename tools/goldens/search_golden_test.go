// Golden exporter for crates/search (capture.md §1, §2.7).
//
// This file is not part of the Go repo. tools/goldens/export.sh injects it
// into I:internal/search with `go test -overlay`. It only runs when
// IRORI_GOLDEN_OUT is set, and writes $IRORI_GOLDEN_OUT/search/search.json:
// ParseRef and Page over corpora, and json.Marshal of Hit and Shown.
package search

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

type goldRef struct {
	Ref  string `json:"ref"`
	Kind string `json:"kind"`
	ID   string `json:"id"`
	Err  string `json:"err"`
}

type goldPage struct {
	Full   string `json:"full"`
	Offset int    `json:"offset"`
	Limit  int    `json:"limit"`
	Text   string `json:"text"`
	More   bool   `json:"more"`
	Next   int    `json:"next"`
}

type goldJSON struct {
	Name string `json:"name"`
	JSON string `json:"json"`
}

func TestGoldenSearch(t *testing.T) {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	var refs []goldRef
	for _, r := range []string{
		"turn:t-1", "TURN:t-1", "Turn:t-1", "edit:e-1", "commit:ABCDEF1", "commit:abcdef1234567890abcdef1234567890abcdef12",
		"commit:abcdef1234567890abcdef1234567890abcdef123", "commit:abc", "commit:zzzzzzz", "commit:ABCDEFG",
		"msg:01JX", "task:01JY", " Task:01JY ", " task:x ", "　task:x", "comment:c1",
		"pr:stephenshorton/irori#12", "pr:o/n#12#3", "pr:irori#12", "pr:a/b#x", "pr:a/b#", "pr:/#1",
		"decision:d1", "agent:dev1/abc-123", "lease:gpu", "task:", "nocolon", "task:a b", "task:a\tb",
		"task:a:b", "taſk:x", "TAKK:x", ":x", "", "agent:d/s x", "commit:K234567",
	} {
		k, id, err := ParseRef(r)
		g := goldRef{Ref: r, Kind: string(k), ID: id}
		if err != nil {
			g.Err = err.Error()
		}
		refs = append(refs, g)
	}
	var pages []goldPage
	for _, full := range []string{strings.Repeat("é", 10), "", "abc", "日本語のテキスト" + strings.Repeat("x", 7000), "a😀b😀c"} {
		for _, o := range []ShowOptions{{}, {Limit: 1}, {Limit: 5}, {Offset: 5, Limit: 4}, {Offset: 99}, {Offset: -3, Limit: 2},
			{Offset: 1, Limit: 3}, {Limit: 7000}, {Offset: 4001}, {Offset: 2, Limit: -1}} {
			text, more, next := Page(full, o)
			pages = append(pages, goldPage{full, o.Offset, o.Limit, text, more, next})
		}
	}
	at := time.Date(2026, 10, 29, 8, 7, 6, 500000000, time.FixedZone("", -6*3600))
	var js []goldJSON
	add := func(name string, v any) {
		b, err := json.Marshal(v)
		if err != nil {
			t.Fatal(err)
		}
		js = append(js, goldJSON{name, string(b)})
	}
	add("hitZero", Hit{})
	add("hitFull", Hit{Ref: "pr:o/n#1", Kind: KindPR, At: at, Title: "PR <o/n>#1 & co", Snippet: "[x]  ", Agent: "d/s", RepoKey: "remote:github.com/o/n"})
	add("shownZero", Shown{})
	add("shownFull", Shown{Ref: "task:1", Kind: KindTask, Title: "t", Text: "<b>", More: true, Next: 4000, Related: []string{"comment:1", "turn:t-2"}})
	add("shownEmptyRelated", Shown{Ref: "msg:1", Kind: KindMsg, Related: []string{}})
	out := map[string]any{"refs": refs, "pages": pages, "json": js}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(out); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "search", "search.json")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

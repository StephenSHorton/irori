// Golden exporter for crates/notify (inbox.md §3.11, architecture.md §6.2).
//
// Injected into I:internal/notify by tools/goldens/export.sh with
// `go test -overlay`; it runs only when IRORI_GOLDEN_OUT is set. It writes
// notify/settings.json: for each case, the settings.json a data folder
// held before (absent, or these bytes), the Options load read from it,
// and, when the case saves, the exact bytes save wrote for its Options.
// crates/notify replays every case and compares bytes and values.
package notify

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

type settingsCase struct {
	Name   string   `json:"name"`
	Before *string  `json:"before"` // nil: no file
	Loaded Options  `json:"loaded"`
	Save   *Options `json:"save,omitempty"`
	After  string   `json:"after,omitempty"`
}

func TestGoldenNotifySettings(t *testing.T) {
	out := os.Getenv("IRORI_GOLDEN_OUT")
	if out == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	str := func(s string) *string { return &s }
	opt := func(o Options) *Options { return &o }
	cases := []settingsCase{
		{Name: "no file, defaults saved", Save: opt(DefaultOptions())},
		{Name: "unknown keys kept and sorted", Before: str(`{"zeta":[1, 2],"other":{"x":1,"h":"<a&b>","u":" "},"alpha":true}`),
			Save: opt(Options{Enabled: false, Sound: false, Volume: 1, Corner: CornerBottomLeft})},
		{Name: "volume fractions", Before: str(`{"notifications":{"enabled":true,"sound":true,"volume":0.25,"corner":"top-left"}}`),
			Save: opt(Options{Enabled: true, Sound: true, Volume: 0.35, Corner: CornerTopLeft})},
		{Name: "tiny volume", Save: opt(Options{Enabled: true, Sound: true, Volume: 0.0000001, Corner: CornerTopRight})},
		{Name: "partial fills from defaults", Before: str(`{"notifications":{"corner":"bottom-left"}}`)},
		{Name: "keys fold case, last wins", Before: str(`{"notifications":{"Corner":"top-left","VOLUME":0.9,"volume":0.2,"Enabled":false}}`)},
		{Name: "out of range normalized", Before: str(`{"notifications":{"volume":3,"corner":"middle"}}`)},
		{Name: "a type error is the defaults", Before: str(`{"notifications":{"volume":"loud","corner":"top-left"}}`)},
		{Name: "notifications null", Before: str(`{"notifications":null}`)},
		{Name: "broken file replaced", Before: str(`{not json`), Save: opt(DefaultOptions())},
		{Name: "not an object replaced", Before: str(`[1,2]`), Save: opt(DefaultOptions())},
		{Name: "duplicate top-level keys", Before: str(`{"a":1,"a":{"b":2},"notifications":{"sound":false}}`),
			Save: opt(Options{Enabled: true, Sound: false, Volume: 0.6, Corner: CornerTopRight})},
		{Name: "pretty file re-indented", Before: str("{\n\t\"other\" : [ 1 ,\n {\"k\" : \"v\"} ] ,\n\t\"notifications\": {\"enabled\": true}\n}\n"),
			Save: opt(Options{Enabled: true, Sound: true, Volume: 0.6, Corner: CornerBottomRight})},
	}
	for i := range cases {
		c := &cases[i]
		dir := t.TempDir()
		if c.Before != nil {
			if err := os.WriteFile(filepath.Join(dir, SettingsFile), []byte(*c.Before), 0o600); err != nil {
				t.Fatal(err)
			}
		}
		c.Loaded = load(dir)
		if c.Save != nil {
			if err := save(dir, *c.Save); err != nil {
				t.Fatal(err)
			}
			b, err := os.ReadFile(filepath.Join(dir, SettingsFile))
			if err != nil {
				t.Fatal(err)
			}
			c.After = string(b)
		}
	}
	path := filepath.Join(out, "notify", "settings.json")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(cases); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

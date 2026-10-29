package presence

import (
	"strings"
	"testing"
)

// fakeNamer gives every session it is asked to name "heron", and remembers.
type fakeNamer map[string]string

func (f fakeNamer) Assigned(id string) string { return f[id] }
func (f fakeNamer) Assign(id string) string {
	if f[id] == "" {
		f[id] = "heron"
	}
	return f[id]
}

func TestNamePrecedence(t *testing.T) {
	cases := []struct {
		what       string
		s          session
		t          tail
		assigned   string
		wantName   string
		wantSource string
	}{
		{"a /rename wins", session{ID: "s", Name: "Bob", NameSource: "user"}, tail{Custom: "Bob", AI: "Fix it"}, "heron", "Bob", SourceUser},
		{"a session name with no source is a choice", session{ID: "s", Name: "api"}, tail{}, "", "api", SourceUser},
		{"a custom title is a choice", session{ID: "s"}, tail{Custom: "parser work", AI: "Fix it"}, "", "parser work", SourceUser},
		{"Irori's name written back as a title stays Irori's", session{ID: "s"}, tail{Custom: "Heron"}, "heron", "Heron", SourceIrori},
		{"Irori's name written back as the session name stays Irori's", session{ID: "s", Name: "heron", NameSource: "user"}, tail{}, "heron", "heron", SourceIrori},
		{"a name Claude set itself isn't a choice", session{ID: "s", Name: "Fix it", NameSource: "auto"}, tail{AI: "Fix it"}, "", "heron", SourceIrori},
		{"an ai-title never is", session{ID: "s", Cwd: `C:\work\api`}, tail{AI: "Fix the parser"}, "", "heron", SourceIrori},
		{"no name at all gets Irori's", session{ID: "s", Cwd: `C:\work\api`}, tail{}, "", "heron", SourceIrori},
	}
	for _, c := range cases {
		f := fakeNamer{}
		if c.assigned != "" {
			f["s"] = c.assigned
		}
		x := New(Options{Names: f}).impl
		name, source := x.name(c.s, c.t)
		if name != c.wantName || source != c.wantSource {
			t.Errorf("%s: got %q (%q), want %q (%q)", c.what, name, source, c.wantName, c.wantSource)
		}
	}
}

func TestNameWithoutNamerKeepsTheOldFallback(t *testing.T) {
	x := New(Options{}).impl
	if name, source := x.name(session{ID: "s", Cwd: `C:\work\api`}, tail{AI: "Fix it", Title: "Fix it"}); name != "Fix it" || source != "" {
		t.Errorf("got %q (%q)", name, source)
	}
	if name, source := x.name(session{ID: "s", Cwd: `C:\work\api`}, tail{}); name != "api" || source != "" {
		t.Errorf("got %q (%q)", name, source)
	}
}

func TestChangedSeesNameSourceAndAITitle(t *testing.T) {
	base := AgentCard{Key: "d/s"}
	b := base
	b.NameSource = SourceIrori
	if !changed(base, b) {
		t.Error("nameSource change ignored")
	}
	b = base
	b.AITitle = "Fix it"
	if !changed(base, b) {
		t.Error("aiTitle change ignored")
	}
}

func TestReadTailKeepsCustomAndAITitlesApart(t *testing.T) {
	src := `{"type":"ai-title","aiTitle":"Fix it"}` + "\n" + `{"type":"custom-title","customTitle":"Bob"}` + "\n"
	got := readTail(strings.NewReader(src), int64(len(src)), 1<<20)
	if got.Custom != "Bob" || got.AI != "Fix it" || got.Title != "Bob" {
		t.Errorf("%+v", got)
	}
}

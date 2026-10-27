package capture

import (
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/capture/transcript"
)

// noisyRepo is a repo ignoring bin/ and *.log, with keep.log tracked anyway.
func noisyRepo(t *testing.T) string {
	t.Helper()
	dir := newRepo(t, "false")
	writeFile(t, filepath.Join(dir, ".gitignore"), "bin/\n*.log\n")
	writeFile(t, filepath.Join(dir, "keep.log"), "tracked\n")
	gitCmd(t, dir, "add", ".gitignore")
	gitCmd(t, dir, "add", "-f", "keep.log")
	gitCmd(t, dir, "commit", "-q", "-m", "ignore")
	return dir
}

// bashDiff is a shell call whose result carries bashEditDiff.changedFiles.
func (w *tw) bashDiff(id, tool string, changed []string, at time.Time) []obj {
	tur := obj{"stdout": "", "stderr": "", "interrupted": false, "isImage": false,
		"bashEditDiff": obj{"files": []any{}, "changedFiles": changed}}
	return []obj{
		w.toolUse(id, tool, obj{"command": "build", "description": "run"}, at),
		w.result(id, tur, at),
	}
}

// editsBy maps logged edit paths by fidelity.
func (r *rig) editsBy() map[string][]string {
	r.t.Helper()
	out := map[string][]string{}
	for _, raw := range r.rows(KindEdit) {
		e := decode[EditRow](r.t, raw)
		out[e.Fidelity] = append(out[e.Fidelity], e.Path)
	}
	for _, v := range out {
		sort.Strings(v)
	}
	return out
}

func TestHeuristicNoiseDropped(t *testing.T) {
	r := newRig(t, nil)
	dir := noisyRepo(t)
	outside := canon(t, t.TempDir()) // no repo
	w := r.session(sidA, dir)
	now := time.Now()
	gen := filepath.Join(dir, "bin", "gen.txt")
	w.write(flat(
		[]obj{w.prompt("p1", "build", now)},
		w.writeTool("w1", gen, "generated\n", nil, now.Add(time.Second)),
		w.bashDiff("b1", "Bash", []string{
			filepath.Join(dir, "foo.go"),            // kept
			filepath.Join(dir, "keep.log"),          // ignored pattern but tracked: kept
			filepath.Join(dir, "bin", "app.txt"),    // ignored
			filepath.Join(dir, "debug.log"),         // ignored
			gen,                                     // ignored, but Write logged it exactly
			filepath.Join(dir, "out", "app.DLL"),    // artifact extension
			filepath.Join(dir, ".git", "index.txt"), // under .git
			filepath.Join(outside, "notes.txt"),     // outside any repo
		}, now.Add(2*time.Second)),
		[]obj{w.reply("done", now.Add(3*time.Second)), w.done(now.Add(4 * time.Second))},
	)...)
	r.tail()

	got := r.editsBy()
	if want := []string{"foo.go", "keep.log"}; !slices.Equal(got[transcript.Heuristic], want) {
		t.Fatalf("heuristic edits = %v, want %v", got[transcript.Heuristic], want)
	}
	if want := []string{"bin/gen.txt"}; !slices.Equal(got[transcript.Exact], want) {
		t.Fatalf("exact edits = %v, want %v", got[transcript.Exact], want)
	}
	if d, want := r.c.Status().Dropped, (Dropped{Artifact: 2, OutsideRepo: 1, Ignored: 3}); d != want {
		t.Fatalf("dropped = %+v, want %+v", d, want)
	}
	turns := r.rows(KindTurn)
	if len(turns) != 1 {
		t.Fatalf("turns = %d", len(turns))
	}
	paths := decode[TurnRow](t, turns[0]).EditPaths
	sort.Strings(paths)
	if want := []string{"bin/gen.txt", "foo.go", "keep.log"}; !slices.Equal(paths, want) {
		t.Fatalf("turn edit paths = %v, want %v", paths, want)
	}
	if len(r.c.impl.noisePaths) != 0 {
		t.Fatalf("noise paths kept after the turn closed: %v", r.c.impl.noisePaths)
	}
}

func TestHeuristicCapPerCall(t *testing.T) {
	r := newRig(t, nil)
	dir := noisyRepo(t)
	w := r.session(sidA, dir)
	now := time.Now()
	var a, b []string
	for i := range maxShellEdits + 10 {
		a = append(a, filepath.Join(dir, fmt.Sprintf("a%02d.go", i)))
	}
	for i := range 5 {
		b = append(b, filepath.Join(dir, fmt.Sprintf("b%02d.go", i)))
	}
	w.write(flat(
		[]obj{w.prompt("p1", "gen", now)},
		w.bashDiff("b1", "PowerShell", a, now.Add(time.Second)),
		w.bashDiff("b2", "Bash", b, now.Add(2*time.Second)),
		[]obj{w.done(now.Add(3 * time.Second))},
	)...)
	r.tail()

	h := r.editsBy()[transcript.Heuristic]
	if len(h) != maxShellEdits+5 {
		t.Fatalf("heuristic edits = %d, want %d", len(h), maxShellEdits+5)
	}
	if slices.Contains(h, fmt.Sprintf("a%02d.go", maxShellEdits)) || !slices.Contains(h, fmt.Sprintf("a%02d.go", maxShellEdits-1)) {
		t.Fatalf("cap kept the wrong edits: %v", h)
	}
	if d := r.c.Status().Dropped; d != (Dropped{OverCap: 10}) {
		t.Fatalf("dropped = %+v", d)
	}
	paths := decode[TurnRow](t, r.rows(KindTurn)[0]).EditPaths
	if len(paths) != maxShellEdits+5 {
		t.Fatalf("turn edit paths = %d, want %d", len(paths), maxShellEdits+5)
	}
}

// A failed check-ignore keeps the edits (and caches nothing); the next call
// asks again.
func TestCheckIgnoreFailsOpen(t *testing.T) {
	g := &flakyGit{sub: "check-ignore", left: 1}
	r := newRig(t, func(o *Options) { o.Git = g })
	dir := noisyRepo(t)
	w := r.session(sidA, dir)
	now := time.Now()
	ign := filepath.Join(dir, "bin", "app.txt")
	w.write(flat(
		[]obj{w.prompt("p1", "build", now)},
		w.bashDiff("b1", "Bash", []string{ign}, now.Add(time.Second)),
		w.bashDiff("b2", "Bash", []string{ign}, now.Add(2*time.Second)),
		[]obj{w.done(now.Add(3 * time.Second))},
	)...)
	r.tail()

	if h := r.editsBy()[transcript.Heuristic]; !slices.Equal(h, []string{"bin/app.txt"}) {
		t.Fatalf("heuristic edits = %v, want the first call's only", h)
	}
	if d := r.c.Status().Dropped; d != (Dropped{Ignored: 1}) {
		t.Fatalf("dropped = %+v", d)
	}
	// Kept by one call, so the turn still lists it.
	if paths := decode[TurnRow](t, r.rows(KindTurn)[0]).EditPaths; len(paths) != 1 {
		t.Fatalf("turn edit paths = %v", paths)
	}
}

func TestIsArtifact(t *testing.T) {
	for p, want := range map[string]bool{
		`C:\r\bin\Debug\App.dll`:       true,
		"/r/assets/model.vmdl_c":       true,
		"/r/assets/model.vmdl":         false,
		"/r/web/node_modules/x/y.js":   true,
		"/r/.git/config":               true,
		"/r/.gitignore":                false,
		"/r/src/main.go":               false,
		"/r/Makefile":                  false,
		"/r/img/logo.PNG":              true,
		`C:\r\.github\workflows\ci.ym`: false,
	} {
		if got := isArtifact(p); got != want {
			t.Errorf("isArtifact(%q) = %v, want %v", p, got, want)
		}
	}
}

// A shell edit inside an agent worktree that has since been deleted must
// not be judged by the parent repo, which ignores its worktrees directory.
func TestHeuristicDeletedWorktreeKept(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	writeFile(t, filepath.Join(dir, ".git", "info", "exclude"), ".claude/\n")
	if err := os.MkdirAll(filepath.Join(dir, ".claude", "worktrees"), 0o755); err != nil {
		t.Fatal(err)
	}
	gone := filepath.Join(dir, ".claude", "worktrees", "wf-gone") // never created
	w := r.session(sidA, gone)
	now := time.Now()
	w.write(flat(
		[]obj{w.prompt("p1", "fmt", now)},
		w.bashDiff("b1", "Bash", []string{filepath.Join(gone, "internal", "main.go")}, now.Add(time.Second)),
		[]obj{w.done(now.Add(2 * time.Second))},
	)...)
	r.tail()

	h := r.editsBy()[transcript.Heuristic]
	if len(h) != 1 || !strings.HasSuffix(filepath.ToSlash(h[0]), "wf-gone/internal/main.go") {
		t.Fatalf("heuristic edits = %v, want the deleted worktree's main.go", h)
	}
	if d := r.c.Status().Dropped; d != (Dropped{}) {
		t.Fatalf("dropped = %+v", d)
	}
}

// A failed rev-parse inside a repo keeps the edit (without a root) rather
// than calling it outside every repo.
func TestHeuristicRevParseFailsOpen(t *testing.T) {
	g := &flakyGit{sub: "rev-parse", left: 1 << 20}
	r := newRig(t, func(o *Options) { o.Git = g })
	dir := newRepo(t, "false")
	outside := canon(t, t.TempDir())
	w := r.session(sidA, dir)
	now := time.Now()
	w.write(flat(
		[]obj{w.prompt("p1", "fmt", now)},
		w.bashDiff("b1", "Bash", []string{
			filepath.Join(dir, "main.go"),
			filepath.Join(outside, "notes.txt"), // no .git anywhere above: still outside
		}, now.Add(time.Second)),
		[]obj{w.done(now.Add(2 * time.Second))},
	)...)
	r.tail()

	if h := r.editsBy()[transcript.Heuristic]; !slices.Equal(h, []string{filepath.Join(dir, "main.go")}) {
		t.Fatalf("heuristic edits = %v, want main.go", h)
	}
	if d := r.c.Status().Dropped; d != (Dropped{OutsideRepo: 1}) {
		t.Fatalf("dropped = %+v", d)
	}
}

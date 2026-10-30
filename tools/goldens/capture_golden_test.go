// Golden exporter for crates/capture's service (capture.md §0, §2.3, §3,
// §6 "capture service").
//
// Injected into I:internal/capture by tools/goldens/export.sh with
// `go test -overlay`, together with capture_harness_test.go (the package's
// harness, which records every rig at the end of its test). It runs every
// Go capture test; when IRORI_GOLDEN_OUT is set, TestMain writes
// $IRORI_GOLDEN_OUT/capture/:
//
//	replay.json  per test: the rows of the rig's log (kind, actor, exact
//	             payload) and Go's views of them (Timeline per agent key,
//	             Edits, Commits, Repos, PRs, and the search listing, a few
//	             ranked searches, Show of each listed ref and agent, the
//	             Overview of each repo) at the recorded now. Rust
//	             replays the rows and must print the same views.
//	tail.json    a synthetic session in a cwd outside every repo, tailed by
//	             Go: its lines, the rows Go logged, the blobs and the cursor
//	             file. Rust tails the same lines and must log the same bytes.
//	vectors.json row ids, normRoot, relTo, commandDir, clipRunes,
//	             isArtifact, editPathKey, the cursor file's bytes.
//
// Every fixture is synthetic; no real transcript content is used.
package capture

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/search"
	"github.com/StephenSHorton/irori/internal/store"
)

type goldRow struct {
	Kind    string `json:"kind"`
	Actor   string `json:"actor"`
	Payload string `json:"payload"`
}

type goldRig struct {
	Test  string            `json:"test"`
	Now   time.Time         `json:"now"`
	Rows  []goldRow         `json:"rows"`
	Views map[string]string `json:"views"`
}

var (
	goldMu   sync.Mutex
	goldRigs []goldRig
)

func goldJSON(v any) string {
	b, err := json.Marshal(v)
	if err != nil {
		panic(err)
	}
	return scrubLocal(string(b))
}

// The local temp and home directories, and what the goldens show instead.
const (
	goldTemp = `C:\tmp`
	goldHome = `C:\Users\someone`
)

// scrubLocal replaces this machine's temp and home directories, as raw
// text and as JSON-escaped text, so no local path is committed. Rows and
// views get the same replacement, and replay never touches the disk.
func scrubLocal(s string) string {
	home, _ := os.UserHomeDir()
	for _, p := range [][2]string{{os.TempDir(), goldTemp}, {home, goldHome}} {
		if p[0] == "" {
			continue
		}
		esc := func(x string) string { return strings.ReplaceAll(x, `\`, `\\`) }
		s = strings.ReplaceAll(s, esc(p[0]), esc(p[1]))
		s = strings.ReplaceAll(s, p[0], p[1])
		s = strings.ReplaceAll(s, filepath.ToSlash(p[0]), filepath.ToSlash(p[1]))
	}
	return s
}

func allRows(r *rig) []store.Event {
	var out []store.Event
	var after store.Cursor
	for {
		evs, err := r.st.List(r.ctx, store.Filter{Kinds: replayKinds, After: after, Limit: store.MaxListLimit})
		if err != nil {
			panic(err)
		}
		out = append(out, evs...)
		if len(evs) < store.MaxListLimit {
			return out
		}
		after = evs[len(evs)-1].Cursor()
	}
}

// recordRig keeps a rig's rows and views. Live cards are cleared first: an
// in-memory open turn is not in the log, so replay can't show it.
func recordRig(r *rig) {
	if os.Getenv("IRORI_GOLDEN_OUT") == "" {
		return
	}
	r.roster.set()
	evs := allRows(r)
	g := goldRig{Test: r.t.Name(), Now: r.clock.now(), Views: map[string]string{}}
	keys := map[string]bool{}
	for _, ev := range evs {
		g.Rows = append(g.Rows, goldRow{ev.Kind, ev.Actor, scrubLocal(string(ev.Payload))})
		if k, ok := strings.CutPrefix(ev.Actor, "agent:"); ok {
			keys[k] = true
		}
	}
	var ks []string
	for k := range keys {
		ks = append(ks, k)
	}
	sort.Strings(ks)
	c := r.c.impl
	// Pin the clock the views read (ledger ages, cmd cut) to the recorded now.
	saved := c.now
	c.now = func() time.Time { return g.Now }
	defer func() { c.now = saved }()
	for _, k := range ks {
		g.Views["timeline "+k] = goldJSON(c.timeline(TimelineQuery{Key: k}))
		g.Views["timeline+sub "+k] = goldJSON(c.timeline(TimelineQuery{Key: k, Subagents: true, Limit: 100}))
		g.Views["prowners "+k] = goldJSON(c.prOwners(k))
	}
	g.Views["edits"] = goldJSON(c.edits(EditQuery{Limit: 500}))
	g.Views["commits"] = goldJSON(c.commits("", 500))
	g.Views["repos"] = goldJSON(c.repos())
	g.Views["prs"] = goldJSON(c.prs(""))
	recordSearch(r, g.Views, ks)
	goldMu.Lock()
	goldRigs = append(goldRigs, g)
	goldMu.Unlock()
}

// goldWords are the ranked searches each rig records.
var goldWords = []string{"add", "foo", "lease", "walrus", "commit"}

// recordSearch adds the search, show and overview views of a rig: the
// newest-first listing of capture's own kinds (a test's fake sources are
// not in the log, so their kinds are left out), a few ranked searches,
// Show of every listed ref and of each agent key, and the overview of each
// repo key seen. The clock is already pinned.
func recordSearch(r *rig, views map[string]string, keys []string) {
	own := []search.Kind{search.KindTurn, search.KindEdit, search.KindCommit, search.KindPR, search.KindDecision}
	hits, err := r.c.Search(r.ctx, SearchQuery{Kinds: own, Limit: search.MaxLimit + 1})
	if err != nil {
		panic(err)
	}
	views["search"] = goldJSON(hits)
	for _, w := range goldWords {
		h, err := r.c.Search(r.ctx, SearchQuery{Text: w, Kinds: own, Limit: search.MaxLimit + 1})
		if err != nil {
			panic(err)
		}
		views["search "+w] = goldJSON(h)
	}
	refs := map[string]bool{}
	for _, h := range hits {
		refs[h.Ref] = true
		// An edit's Show prints its patch: keep the blob for the replay.
		if id, ok := strings.CutPrefix(h.Ref, "edit:"); ok {
			if e := r.c.impl.editByID[id]; e != nil && e.Patch != "" {
				if b, err := r.c.impl.blobs.Get(e.Patch); err == nil {
					views["blob "+e.Patch] = scrubLocal(string(b))
				}
			}
		}
	}
	for _, k := range keys {
		refs["agent:"+k] = true
	}
	for ref := range refs {
		if sh, err := r.c.Show(r.ctx, ref, ShowOptions{}); err == nil {
			views["show "+ref] = goldJSON(sh)
		}
	}
	for rk := range r.c.impl.repos() {
		views["overview "+rk] = goldJSON(r.c.Overview(r.ctx, "", OverviewInput{RepoKey: rk}))
	}
}

var goldOut = map[string]any{}

func TestMain(m *testing.M) {
	code := m.Run()
	if dir := os.Getenv("IRORI_GOLDEN_OUT"); dir != "" && code == 0 {
		sort.SliceStable(goldRigs, func(i, j int) bool { return goldRigs[i].Test < goldRigs[j].Test })
		goldOut["replay.json"] = goldRigs
		for name, v := range goldOut {
			var buf bytes.Buffer
			enc := json.NewEncoder(&buf)
			enc.SetEscapeHTML(false)
			enc.SetIndent("", " ")
			if err := enc.Encode(v); err != nil {
				panic(err)
			}
			path := filepath.Join(dir, "capture", name)
			if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
				panic(err)
			}
			if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
				panic(err)
			}
		}
	}
	os.Exit(code)
}

// goldBase is the synthetic sessions' clock.
var goldBase = time.Date(2026, 10, 27, 9, 0, 0, 0, time.UTC)

func gat(sec int) time.Time { return goldBase.Add(time.Duration(sec) * time.Second) }

// TestGoldenCaptureTail tails synthetic sessions whose cwd is on a drive
// that doesn't exist, so no git runs and every path is fixed.
func TestGoldenCaptureTail(t *testing.T) {
	if os.Getenv("IRORI_GOLDEN_OUT") == "" || runtime.GOOS != "windows" {
		t.Skip("IRORI_GOLDEN_OUT not set, or not Windows")
	}
	const cwd = `Q:\irori-golden\demo`
	r := newRig(t, nil)
	type file struct {
		Rel   string   `json:"rel"`
		Lines []string `json:"lines"`
	}
	var files []file
	write := func(w *tw, rows ...obj) {
		var lines []string
		for _, m := range rows {
			lines = append(lines, string(lineOf(t, m)))
		}
		w.write(rows...)
		rel, _ := filepath.Rel(r.claude, w.path)
		files = append(files, file{Rel: rel, Lines: lines})
	}
	orig := "package demo\r\n\r\nfunc A() {}\r\n"
	long := strings.Repeat("summary words ", 200)
	a := r.session("aaaaaaaa-1111-2222-3333-444444444444", cwd)
	write(a,
		a.prompt("p1", "fix the <thing> & the build", gat(0)),
		a.reply("Looking.", gat(1)),
		a.toolUse("e1", "Edit", obj{"file_path": cwd + `\a.go`, "old_string": "func A() {}", "new_string": "func A() { b() }"}, gat(2)),
		a.result("e1", obj{"filePath": cwd + `\a.go`, "oldString": "func A() {}", "newString": "func A() { b() }",
			"originalFile": orig, "replaceAll": false, "userModified": false,
			"structuredPatch": []any{obj{"oldStart": 3, "oldLines": 1, "newStart": 3, "newLines": 1,
				"lines": []string{"-func A() {}", "+func A() { b() }"}}}}, gat(3)),
		a.toolUse("w1", "Write", obj{"file_path": `sub/new.txt`, "content": "one\ntwo\n"}, gat(4)),
		a.result("w1", obj{"type": "create", "filePath": cwd + `\sub\new.txt`, "content": "one\ntwo\n",
			"structuredPatch": []any{}, "originalFile": nil}, gat(5)),
		a.toolUse("b1", "Bash", obj{"command": "cd Q:/irori-golden/demo && git commit -am \"fix\" && gh pr create --fill", "description": "commit"}, gat(6)),
		a.result("b1", obj{"stdout": "[main 1234567] fix\nhttps://github.com/acme/demo/pull/42\n", "stderr": "",
			"interrupted": false, "isImage": false}, gat(8)),
		a.toolUse("b2", "Bash", obj{"command": "make", "description": "build"}, gat(9)),
		a.result("b2", obj{"stdout": "", "stderr": "", "interrupted": false,
			"bashEditDiff": obj{"files": []any{obj{"filePath": cwd + `\gen.go`, "hunks": []any{obj{"oldStart": 1, "oldLines": 0, "newStart": 1, "newLines": 1, "lines": []string{"+x"}}}}},
				"changedFiles": []string{cwd + `\gen.go`, cwd + `\out\app.dll`}}}, gat(10)),
		a.done(gat(11)),
		a.common("ai-title", gat(11), obj{"aiTitle": "Fix the build"}),
		a.common("system", gat(12), obj{"subtype": "away_summary", "content": long}),
		a.common("pr-link", gat(13), obj{"prNumber": "#42", "prUrl": "https://github.com/acme/demo/pull/42", "prRepository": "acme/demo"}),
		a.prompt("p2", "now the tests", gat(20)),
		a.common("user", gat(21), obj{"message": obj{"role": "user", "content": "[Request interrupted by user]"}}),
		a.prompt("p3", "<command-name>/clear</command-name>", gat(30)),
		a.done(gat(31)),
		a.common("system", gat(32), obj{"subtype": "compact_boundary", "content": "Conversation compacted"}),
		a.common("user", gat(33), obj{"isCompactSummary": true, "message": obj{"role": "user", "content": "The summary."}}),
		a.prompt("p4", "still open at the end", gat(40)),
	)
	s := r.subagent("aaaaaaaa-1111-2222-3333-444444444444", cwd, "agentX1")
	write(s,
		s.prompt("sp1", "do the subtask", gat(14)),
		s.toolUse("n1", "NotebookEdit", obj{"notebook_path": cwd + `\n.ipynb`, "new_source": "x"}, gat(15)),
		s.result("n1", obj{"notebook_path": cwd + `\n.ipynb`, "cell_id": "c1", "new_source": "x",
			"structuredPatch": []any{obj{"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 1, "lines": []string{"-y", "+x"}}}}, gat(16)),
		s.done(gat(17)),
	)
	b := r.session("bbbbbbbb-1111-2222-3333-444444444444", `Q:\irori-golden\other`)
	write(b,
		b.common("custom-title", gat(50), obj{"customTitle": "Other work"}),
		b.common("user", gat(51), obj{"promptId": "q1", "origin": obj{"kind": "plugin", "name": "tsukumo"},
			"message": obj{"role": "user", "content": []any{obj{"type": "text", "text": "a"}, obj{"type": "text", "text": "b"}}}}),
		b.toolUse("x1", "PowerShell", obj{"command": "git -C ..\\demo commit -m x"}, gat(52)),
		b.result("x1", "Error: exit 1", gat(53)),
		b.done(gat(54)),
	)
	r.tail()
	rows := func() []goldRow {
		var out []goldRow
		for _, ev := range allRows(r) {
			out = append(out, goldRow{ev.Kind, ev.Actor, string(ev.Payload)})
		}
		return out
	}
	first := rows()
	blobs := map[string]string{}
	for _, g := range first {
		var m map[string]any
		_ = json.Unmarshal([]byte(g.Payload), &m)
		for _, k := range []string{"patch", "fullBlob"} {
			if id, ok := m[k].(string); ok && id != "" {
				b, err := r.c.Blob(id)
				if err != nil {
					t.Fatal(err)
				}
				blobs[id] = string(b)
			}
		}
	}
	cursors, err := os.ReadFile(filepath.Join(r.data, cursorFileName))
	if err != nil {
		t.Fatal(err)
	}
	r.restart()
	r.tail()
	again := rows()
	goldOut["tail.json"] = map[string]any{
		"device": device, "files": files, "rows": first, "blobs": blobs,
		"cursors": strings.ReplaceAll(string(cursors), strings.ReplaceAll(r.claude, `\`, `\\`), "$CLAUDE"),
		"rowsAfterRestart": len(again), "status": r.c.Status().Dropped,
	}
}

// TestGoldenCaptureVectors records the pure helpers.
func TestGoldenCaptureVectors(t *testing.T) {
	if os.Getenv("IRORI_GOLDEN_OUT") == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	v := map[string]any{}
	type idv struct {
		Kind  string   `json:"kind"`
		Parts []string `json:"parts"`
		ID    string   `json:"id"`
	}
	var ids []idv
	for _, p := range [][3]string{
		{"dev1/s1", "", "p1"}, {"dev1/s1", "agent1", "p1"}, {"", "", ""}, {"0123456789abcdef0123456789abcdef/abc", "", "8d2e-é"},
	} {
		ids = append(ids, idv{"turn", p[:], turnRowID(p[0], p[1], p[2])})
	}
	for _, p := range [][3]string{
		{"dev1/s1", "toolu_1", `C:\src\demo\a.go`}, {"dev1/s1", "toolu_1", "sub/new.txt"}, {"k", "", ""},
	} {
		ids = append(ids, idv{"edit", p[:], editRowID(p[0], p[1], p[2])})
	}
	for _, p := range [][4]string{
		{"dev1/s1", "", "ai_title", "Fix the build"}, {"dev1/s1", "agent1", "away_summary", "uuid-1"}, {"k|x", "a|b", "c", "d"},
	} {
		ids = append(ids, idv{"summary", p[:], summaryRowID(p[0], p[1], p[2], p[3])})
	}
	v["ids"] = ids

	home, _ := os.UserHomeDir()
	v["home"] = scrubLocal(home)
	cwd := `C:\Users\me`
	type cd struct{ Cmd, Cwd, Dir string }
	var cds []cd
	for _, c := range []string{
		`git commit -m x`, `git -C C:/Users/me/repo commit -m x`,
		`git -C "C:\Users\me\my repo" add . && git -C "C:\Users\me\my repo" commit -m x`,
		`git -c user.name=a -C C:/r commit`, `git --no-pager -C C:/r commit`, `git -C sub commit -m x`,
		`cd C:/r && git commit -m x`, `Set-Location -Path 'C:\r'; git commit -m x`, `echo cd later`,
		`git add foo.go && git commit -m 'docs: use git -C C:/elsewhere'`,
		`git commit -m "cd C:/elsewhere && git -C C:/x commit"`, `cd sub && git -C .. commit`,
		`cd ~/projects/repo && git commit -m x`, `cd $HOME/projects/repo && git commit`,
		`cd /c/Users/me/repo && git commit`, `git -C my\ repo commit`, `git -C C:/a log && git -C C:/b commit`,
		`pushd - && git commit`, `cd %USERPROFILE%\x; git commit`, `cd $env:USERPROFILE\y | git commit`,
		`cd /d && git commit`, `C:\Git\bin\GIT.EXE -C C:/z commit`, `cd "C:\a b" || git commit`, `cd ~`,
		"cd C:/x\ngit commit", `sl -LiteralPath C:\q; git merge x`,
	} {
		cds = append(cds, cd{c, cwd, scrubLocal(commandDir(c, cwd))})
	}
	cds = append(cds, cd{`cd sub && git commit`, "", commandDir(`cd sub && git commit`, "")})
	v["commandDir"] = cds

	type pv struct{ In, Norm, Key string }
	var paths []pv
	for _, p := range []string{`C:\Users\Me\Repo`, `c:/users/me/repo/`, `C:\A\..\B`, `/home/x/./y`, `C:\ÄÖ\İx`, ``} {
		paths = append(paths, pv{p, normRoot(p), editPathKey(p)})
	}
	v["paths"] = paths

	type rv struct {
		Root, Path, Rel string
		OK              bool
	}
	var rels []rv
	for _, c := range [][2]string{
		{`C:\r`, `C:\r\a\b.go`}, {`C:\r`, `c:/R/a.go`}, {`C:\r`, `C:\r`}, {`C:\r`, `C:\r2\a`}, {`C:\r`, `D:\r\a`},
		{"", `C:\r`}, {`C:\r`, ""}, {`C:\r\`, `C:\r\x\..\y`}, {`/r`, `/r/a`}, {`C:\r`, `C:\r\..\r\z`},
	} {
		rel, ok := relTo(c[0], c[1])
		rels = append(rels, rv{c[0], c[1], rel, ok})
	}
	v["relTo"] = rels

	type cr struct {
		S    string
		N    int
		Clip string
	}
	var clips []cr
	for _, c := range []struct {
		s string
		n int
	}{{"short", 300}, {strings.Repeat("é", 301), 300}, {"abc def", 4}, {"abc", 3}} {
		clips = append(clips, cr{c.s, c.n, clipRunes(c.s, c.n)})
	}
	v["clipRunes"] = clips

	art := map[string]bool{}
	for _, p := range []string{`C:\r\bin\Debug\App.dll`, "/r/assets/model.vmdl_c", "/r/assets/model.vmdl",
		"/r/web/node_modules/x/y.js", "/r/.git/config", "/r/.gitignore", "/r/src/main.go", "/r/Makefile",
		"/r/img/logo.PNG", `C:\r\.github\workflows\ci.ym`, "/r/.dll", "/r/a.tar.gz", `C:\r\NODE_MODULES\x`, "/r/x.SAV"} {
		art[p] = isArtifact(p)
	}
	v["isArtifact"] = art

	cf := &cursorFile{
		Files:   map[string]int64{`C:\Users\me\.claude\projects\C--x\s1.jsonl`: 123, `C:\Users\me\.claude\projects\C--x\s<&>é.jsonl`: 0},
		Reflogs: map[string]int64{`c:\users\me\projects\repo`: 9876543210},
	}
	b, _ := json.Marshal(cf)
	v["cursorFile"] = string(b)
	goldOut["vectors.json"] = v
}

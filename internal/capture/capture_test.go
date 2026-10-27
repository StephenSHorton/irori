package capture

import (
	"context"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/capture/gitlink"
	"github.com/StephenSHorton/irori/internal/decision"
	"github.com/StephenSHorton/irori/internal/ledger"
)

const (
	sidA = "aaaaaaaa-1111-2222-3333-444444444444"
	sidB = "bbbbbbbb-1111-2222-3333-444444444444"
)

// editFlow is one session that writes foo.go in dir and commits it with the
// given shell tool, tailing in pieces along the way. It returns the commit.
func editFlow(t *testing.T, r *rig, w *tw, dir, content, tool string) string {
	t.Helper()
	foo := filepath.Join(dir, "foo.go")
	now := time.Now()
	w.writeInPieces(r, flat(
		[]obj{w.prompt("p1", "add foo", now)},
		w.writeTool("w1", foo, content, nil, now.Add(time.Second)),
	)...)
	r.repos() // the root is known from the edit: start watching its reflog
	gitCmd(t, dir, "add", "foo.go")
	gitCmd(t, dir, "commit", "-q", "-m", "add foo")
	sha := head(t, dir)
	cmd := "git add foo.go && git commit -m 'add foo'"
	if tool == "PowerShell" {
		cmd = "git add foo.go; if ($?) { git commit -m @'\nadd foo\n'@ }"
	}
	at := time.Now()
	w.writeInPieces(r, flat(
		w.shell("b1", tool, cmd, "[main abc] add foo\n", nil, at),
		[]obj{w.reply("done", at), w.done(at.Add(time.Second))},
	)...)
	r.repos()
	return sha
}

func TestEndToEnd(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	w := r.session(sidA, dir)
	sha := editFlow(t, r, w, dir, "package foo\n\nfunc A() {}\n", "Bash")
	key := device + "/" + sidA

	turns := r.rows(KindTurn)
	if len(turns) != 1 {
		t.Fatalf("turns = %d", len(turns))
	}
	tr := decode[TurnRow](t, turns[0])
	if tr.Key != key || tr.TurnID != "p1" || tr.Prompt != "add foo" || tr.Reply != "done" ||
		tr.EndReason != "turn_duration" || !reflect.DeepEqual(tr.EditPaths, []string{"foo.go"}) || tr.RepoKey == "" {
		t.Fatalf("turn = %+v", tr)
	}
	edits := r.c.Edits(EditQuery{})
	if len(edits) != 1 {
		t.Fatalf("edits = %+v", edits)
	}
	e := edits[0]
	if e.Path != "foo.go" || !strings.EqualFold(e.Root, dir) || e.AfterBlob == "" || e.Patch == "" || e.Fidelity != "exact" || !e.Create || e.Adds != 3 {
		t.Fatalf("edit = %+v", e)
	}
	if p, err := r.c.Blob(e.Patch); err != nil || !strings.Contains(string(p), "+func A() {}") {
		t.Fatalf("patch = %q, %v", p, err)
	}
	cv, ok := r.c.Commit(sha)
	if !ok || cv.Link == nil {
		t.Fatalf("commit %s not linked: %+v", sha, cv)
	}
	if cv.Op != "commit" || cv.Branch != "main" || cv.Subject != "add foo" {
		t.Fatalf("commit = %+v", cv.CommitRow)
	}
	want := []gitlink.FileLink{{Path: "foo.go", Link: gitlink.LinkExact, Edits: []string{e.ID}}}
	if !reflect.DeepEqual(cv.Link.Files, want) || !reflect.DeepEqual(cv.Link.Sessions, []string{key}) ||
		!reflect.DeepEqual(cv.Link.Turns, []string{"p1"}) {
		t.Fatalf("link = %+v", cv.Link)
	}
	if cv.Link.Committer == nil || cv.Link.Committer.Key != key || cv.Link.Committer.TurnID != "p1" {
		t.Fatalf("committer = %+v", cv.Link.Committer)
	}
	if got := r.c.Commits(e.RepoKey, 0); len(got) != 1 || got[0].SHA != sha {
		t.Fatalf("commits = %+v", got)
	}
	if got := r.c.Repos()[e.RepoKey]; len(got) != 1 || !strings.EqualFold(got[0], dir) {
		t.Fatalf("repos = %+v", r.c.Repos())
	}

	tl := r.c.Timeline(TimelineQuery{Key: key})
	// The commit's time is whole seconds, so it may sort on either side.
	kinds := map[EntryKind]*Entry{}
	for i := range tl.Entries {
		kinds[tl.Entries[i].Kind] = &tl.Entries[i]
	}
	if len(tl.Entries) != 2 || kinds[EntryCommit] == nil || kinds[EntryTurn] == nil ||
		len(kinds[EntryTurn].Turn.Edits) != 1 || kinds[EntryCommit].Commit.SHA != sha || tl.More {
		t.Fatalf("timeline = %+v", tl)
	}

	// Why on a committed line names the commit and the turn.
	why, err := r.c.Why(r.ctx, WhyQuery{Path: "foo.go", Line: 3, Cwd: dir})
	if err != nil {
		t.Fatal(err)
	}
	if why.BlameSHA != sha || why.Commit == nil || len(why.Edits) != 1 || len(why.Turns) != 1 || why.Turns[0].Turn.ID != "p1" || why.Path != "foo.go" {
		t.Fatalf("why = %+v", why)
	}

	// An uncommitted line: the edits since the last commit.
	now := time.Now()
	orig := "package foo\n\nfunc A() {}\n"
	w.writeInPieces(r, flat(
		[]obj{w.prompt("p2", "add B", now)},
		w.writeTool("w2", filepath.Join(dir, "foo.go"), orig+"\nfunc B() {}\n", &orig, now),
		[]obj{w.done(now.Add(time.Second))},
	)...)
	why, err = r.c.Why(r.ctx, WhyQuery{Path: filepath.Join(dir, "foo.go"), Line: 5})
	if err != nil {
		t.Fatal(err)
	}
	if why.BlameSHA != "" || why.Commit != nil || len(why.Edits) != 1 || why.Edits[0].TurnID != "p2" || len(why.Turns) != 1 {
		t.Fatalf("uncommitted why = %+v", why)
	}
	if _, err := r.c.Why(r.ctx, WhyQuery{Path: "foo.go"}); err != ErrNoRepo {
		t.Fatalf("no cwd: err = %v", err)
	}
}

func TestCRLFLinksExact(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "true")
	sha := editFlow(t, r, r.session(sidA, dir), dir, "package foo\r\n\r\nfunc A() {}\r\n", "Bash")
	cv, ok := r.c.Commit(sha)
	if !ok || cv.Link == nil || len(cv.Link.Files) != 1 || cv.Link.Files[0].Link != gitlink.LinkExact {
		t.Fatalf("commit = %+v", cv)
	}
	e := r.c.Edits(EditQuery{})[0]
	if e.AfterBlobLF == "" || e.AfterBlobLF != cv.Files[0].Blob || e.AfterBlob == cv.Files[0].Blob {
		t.Fatalf("blobs: edit %s/%s commit %s", e.AfterBlob, e.AfterBlobLF, cv.Files[0].Blob)
	}
}

func TestPowerShellCommitter(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	sha := editFlow(t, r, r.session(sidA, dir), dir, "package foo\n", "PowerShell")
	cmds := r.rows(KindCommitCmd)
	if len(cmds) != 1 {
		t.Fatalf("commit cmds = %d", len(cmds))
	}
	if cmd := decode[CommitCmdRow](t, cmds[0]); cmd.Command != "git add foo.go; if ($?) { git commit -m @'" || cmd.TurnID != "p1" {
		t.Fatalf("cmd = %+v", cmd)
	}
	cv, _ := r.c.Commit(sha)
	if cv.Link == nil || cv.Link.Committer == nil || cv.Link.Committer.Key != device+"/"+sidA {
		t.Fatalf("link = %+v", cv.Link)
	}
}

// A workflow agent commits in a linked worktree, the worktree is removed,
// and main fast-forwards to its branch: main's reflog brings the commit in
// and it links to the edit by blob.
func TestFastForwardFromRemovedWorktree(t *testing.T) {
	r := newRig(t, nil)
	mainDir := newRepo(t, "false")
	wt := filepath.Join(canon(t, t.TempDir()), "wt")
	gitCmd(t, mainDir, "worktree", "add", "-q", "-b", "wf", wt)
	r.roster.set(liveCard(sidB, mainDir, "idle"))

	w := r.session(sidA, wt)
	now := time.Now()
	w.write(flat(
		[]obj{w.prompt("p1", "work", now)},
		w.writeTool("w1", filepath.Join(wt, "pkg", "foo.go"), "package pkg\n", nil, now),
		[]obj{w.done(now.Add(time.Second))},
	)...)
	r.tail()
	r.repos() // watching main (live card) and wt (edit root)
	if n := len(r.c.Repos()); n != 1 {
		t.Fatalf("repos = %+v", r.c.Repos())
	}

	gitCmd(t, wt, "add", "pkg/foo.go")
	gitCmd(t, wt, "commit", "-q", "-m", "wf work")
	sha := head(t, wt)
	gitCmd(t, mainDir, "worktree", "remove", wt)
	gitCmd(t, mainDir, "merge", "-q", "--ff-only", "wf")
	r.repos()

	cv, ok := r.c.Commit(sha)
	if !ok || cv.Link == nil {
		t.Fatalf("commit not seen: %+v", r.c.Commits("", 0))
	}
	if !strings.EqualFold(cv.Root, mainDir) || !strings.HasPrefix(cv.Op, "merge") {
		t.Fatalf("commit = %+v", cv.CommitRow)
	}
	if f := cv.Link.Files; len(f) != 1 || f[0].Link != gitlink.LinkExact || f[0].Path != "pkg/foo.go" || len(f[0].Edits) != 1 {
		t.Fatalf("files = %+v", f)
	}
	if !reflect.DeepEqual(cv.Link.Sessions, []string{device + "/" + sidA}) {
		t.Fatalf("sessions = %v", cv.Link.Sessions)
	}
}

// Two worktrees edit the same path differently; each commit links only its
// own worktree's edit.
func TestTwoWorktreesSamePath(t *testing.T) {
	r := newRig(t, nil)
	mainDir := newRepo(t, "false")
	wt := filepath.Join(canon(t, t.TempDir()), "wt2")
	gitCmd(t, mainDir, "worktree", "add", "-q", "-b", "other", wt)

	a, b := r.session(sidA, mainDir), r.session(sidB, wt)
	now := time.Now()
	a.write(flat([]obj{a.prompt("pa", "a", now)}, w2(a, "wa", filepath.Join(mainDir, "foo.go"), "A\n", now), []obj{a.done(now)})...)
	b.write(flat([]obj{b.prompt("pb", "b", now)}, w2(b, "wb", filepath.Join(wt, "foo.go"), "B\n", now), []obj{b.done(now)})...)
	r.tail()
	r.repos()

	gitCmd(t, mainDir, "add", "foo.go")
	gitCmd(t, mainDir, "commit", "-q", "-m", "a")
	shaA := head(t, mainDir)
	gitCmd(t, wt, "add", "foo.go")
	gitCmd(t, wt, "commit", "-q", "-m", "b")
	shaB := head(t, wt)
	r.repos()

	for _, tc := range []struct{ sha, sid string }{{shaA, sidA}, {shaB, sidB}} {
		cv, ok := r.c.Commit(tc.sha)
		if !ok || cv.Link == nil || cv.Link.Files[0].Link != gitlink.LinkExact ||
			!reflect.DeepEqual(cv.Link.Sessions, []string{device + "/" + tc.sid}) {
			t.Fatalf("commit %s: %+v", tc.sha, cv.Link)
		}
	}
}

func w2(w *tw, id, path, content string, at time.Time) []obj {
	return w.writeTool(id, path, content, nil, at)
}

func TestRestartNoDuplicates(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	w := r.session(sidA, dir)
	editFlow(t, r, w, dir, "package foo\n", "Bash")
	// An open turn at shutdown: re-read whole after the restart.
	now := time.Now()
	w.write(flat([]obj{w.prompt("p2", "more", now)}, w2(w, "w2", filepath.Join(dir, "bar.go"), "package foo\n", now))...)
	r.tail()
	before := r.counts()

	r.restart()
	r.tail()
	r.repos()
	if after := r.counts(); !reflect.DeepEqual(before, after) {
		t.Fatalf("rows changed across a restart:\n%v\n%v", before, after)
	}
	// The open turn closes whole once it ends, prompt included.
	w.write(w.done(time.Now()))
	r.tail()
	var got *TurnRow
	for _, raw := range r.rows(KindTurn) {
		tr := decode[TurnRow](t, raw)
		if tr.TurnID == "p2" {
			got = &tr
		}
	}
	if got == nil || got.Prompt != "more" || got.Origin != "human" || !reflect.DeepEqual(got.EditPaths, []string{"bar.go"}) {
		t.Fatalf("p2 = %+v", got)
	}
}

func TestCrashBeforeCursorReplaysWithoutDuplicates(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	w := r.session(sidA, dir)
	editFlow(t, r, w, dir, "package foo\n", "Bash")
	before := r.counts()

	// The rows were appended but the cursors never saved.
	cf, err := loadCursors(r.data)
	if err != nil {
		t.Fatal(err)
	}
	for p := range cf.Files {
		cf.Files[p] = 0
	}
	// And the reflog cursor stopped before the commit's line.
	b, err := os.ReadFile(filepath.Join(dir, ".git", "logs", "HEAD"))
	if err != nil {
		t.Fatal(err)
	}
	last := strings.LastIndexByte(strings.TrimSuffix(string(b), "\n"), '\n') + 1
	if len(cf.Reflogs) != 1 {
		t.Fatalf("reflogs = %v", cf.Reflogs)
	}
	for p := range cf.Reflogs {
		cf.Reflogs[p] = int64(last)
	}
	cf.dirty = true
	if err := cf.save(r.data); err != nil {
		t.Fatal(err)
	}
	r.restart()
	r.tail()
	r.repos()
	if after := r.counts(); !reflect.DeepEqual(before, after) {
		t.Fatalf("replay duplicated rows:\n%v\n%v", before, after)
	}
}

func TestBackfillRules(t *testing.T) {
	const max = 4096
	r := newRig(t, func(o *Options) { o.BackfillMaxBytes = max })
	cwd := canon(t, t.TempDir())
	old := time.Now().Add(-100 * time.Hour)

	// prompts writes n closed turns with ids prefix0..n-1.
	turnsOf := func(w *tw, prefix string, n int, at time.Time) []obj {
		var rows []obj
		for i := 0; i < n; i++ {
			rows = append(rows, w.prompt(prefix+itoa(i), strings.Repeat("x", 300), at), w.done(at))
		}
		return rows
	}
	recent := r.session("recent0-1111-2222-3333-444444444444", cwd)
	recent.write(turnsOf(recent, "r", 2, old)...)
	stale := r.session("stale000-1111-2222-3333-444444444444", cwd)
	stale.write(turnsOf(stale, "s", 2, old)...)
	if err := os.Chtimes(stale.path, old, old); err != nil {
		t.Fatal(err)
	}
	bigDead := r.session("bigdead0-1111-2222-3333-444444444444", cwd)
	bigDead.write(turnsOf(bigDead, "d", 12, old)...)
	bigLive := r.session("biglive0-1111-2222-3333-444444444444", cwd)
	bigLive.write(turnsOf(bigLive, "b", 12, old)...)
	bigLive.write(bigLive.common("system", old, obj{"subtype": "compact_boundary", "content": "Conversation compacted"}))
	bigLive.write(turnsOf(bigLive, "c", 2, old)...)
	noBoundary := r.session("nobound0-1111-2222-3333-444444444444", cwd)
	noBoundary.write(turnsOf(noBoundary, "n", 12, old)...)
	r.roster.set(liveCard(bigLive.sid, cwd, "idle"), liveCard(noBoundary.sid, cwd, "idle"))

	r.tail()
	got := map[string][]string{}
	for _, raw := range r.rows(KindTurn) {
		tr := decode[TurnRow](t, raw)
		got[tr.Turn.SessionID] = append(got[tr.Turn.SessionID], tr.TurnID)
	}
	if !reflect.DeepEqual(got[recent.sid], []string{"r0", "r1"}) {
		t.Fatalf("recent = %v", got[recent.sid])
	}
	if len(got[stale.sid]) != 0 || len(got[bigDead.sid]) != 0 {
		t.Fatalf("stale/big dead files were backfilled: %v", got)
	}
	if !reflect.DeepEqual(got[bigLive.sid], []string{"c0", "c1"}) {
		t.Fatalf("big live from its compact boundary = %v", got[bigLive.sid])
	}
	nb := got[noBoundary.sid]
	if len(nb) == 0 || len(nb) >= 12 || nb[len(nb)-1] != "n11" {
		t.Fatalf("no-boundary file = %v", nb)
	}
	sums := r.rows(KindSummary)
	if len(sums) != 1 || decode[SummaryRow](t, sums[0]).Kind != "compact_boundary" {
		t.Fatalf("summaries = %d", len(sums))
	}

	// Files that started at their end still pick up new rows.
	now := time.Now()
	stale.write(stale.prompt("s9", "new", now), stale.done(now))
	bigDead.write(bigDead.prompt("d9", "new", now), bigDead.done(now))
	r.tail()
	n := 0
	for _, raw := range r.rows(KindTurn) {
		if id := decode[TurnRow](t, raw).TurnID; id == "s9" || id == "d9" {
			n++
		}
	}
	if n != 2 {
		t.Fatalf("new rows after the end start: %d", n)
	}
	if st := r.c.Status(); st.Backfilling || st.Files != 5 || st.Backlog != 0 || st.LastScan.IsZero() {
		t.Fatalf("status = %+v", st)
	}
}

func TestSubagentAttribution(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	main := r.session(sidA, dir)
	sub := r.subagent(sidA, dir, "a1b2c3")
	now := time.Now()
	main.write(main.prompt("p1", "delegate", now), main.done(now))
	sub.write(flat([]obj{sub.prompt("p1", "do it", now.Add(time.Second))},
		w2(sub, "s1", filepath.Join(dir, "sub.go"), "package sub\n", now.Add(time.Second)),
		[]obj{sub.done(now.Add(2 * time.Second))})...)
	r.tail()
	key := device + "/" + sidA
	es := r.c.Edits(EditQuery{Key: key})
	if len(es) != 1 || es[0].AgentID != "a1b2c3" || es[0].Key != key || es[0].Path != "sub.go" {
		t.Fatalf("edits = %+v", es)
	}
	if n := len(r.rows(KindTurn)); n != 2 {
		t.Fatalf("turns = %d (the subagent's same promptId must not collide)", n)
	}
	if tl := r.c.Timeline(TimelineQuery{Key: key}); len(tl.Entries) != 1 || tl.Entries[0].Turn.AgentID != "" {
		t.Fatalf("timeline without subagents = %+v", tl.Entries)
	}
	tl := r.c.Timeline(TimelineQuery{Key: key, Subagents: true})
	if len(tl.Entries) != 2 || tl.Entries[0].Turn.AgentID != "a1b2c3" || len(tl.Entries[0].Turn.Edits) != 1 {
		t.Fatalf("timeline with subagents = %+v", tl.Entries)
	}
}

func TestPRLedger(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	gitCmd(t, dir, "remote", "add", "origin", "https://github.com/Owner/Repo.git")
	w := r.session(sidA, dir)
	now := time.Now()
	w.write(flat(
		[]obj{w.prompt("p1", "open a pr", now)},
		w.shell("b1", "Bash", "gh pr create --fill", "https://github.com/Owner/Repo/pull/13\n",
			obj{"pr": obj{"number": 13, "url": "https://github.com/Owner/Repo/pull/13", "action": "created"}}, now),
		w.shell("b2", "PowerShell", "gh pr review 7 --approve", "", nil, now.Add(time.Second)),
		[]obj{
			{"type": "pr-link", "sessionId": sidA, "prNumber": "12", "prUrl": "https://github.com/Owner/Repo/pull/12", "prRepository": "Owner/Repo", "timestamp": ts(now)},
			w.done(now.Add(2 * time.Second)),
		},
	)...)
	r.tail()
	key := device + "/" + sidA
	if got := r.c.PROwners(ledger.Key("owner/repo", 13)); !reflect.DeepEqual(got, []string{key}) {
		t.Fatalf("owners #13 = %v", got)
	}
	if got := r.c.PROwners("pr:owner/repo#7"); !reflect.DeepEqual(got, []string{key}) {
		t.Fatalf("owners #7 (review, default repo from the remote) = %v", got)
	}
	prs := r.c.PRs("owner/repo")
	roles := map[int]ledger.Role{}
	for _, e := range prs {
		roles[e.Number] = e.Role
	}
	if !reflect.DeepEqual(roles, map[int]ledger.Role{13: ledger.Author, 12: ledger.Touched, 7: ledger.Reviewer}) {
		t.Fatalf("prs = %+v", prs)
	}
	for _, ev := range r.events(KindPR) {
		e := decode[ledger.Entry](t, ev.Payload)
		want := "agent:" + key
		if e.Source == "pr-link" {
			want = "irori"
		}
		if ev.Actor != want {
			t.Fatalf("pr %d actor = %s", e.Number, ev.Actor)
		}
	}
	tl := r.c.Timeline(TimelineQuery{Key: key})
	n := 0
	for _, e := range tl.Entries {
		if e.Kind == EntryPR {
			n++
		}
	}
	if n != 3 {
		t.Fatalf("timeline PR entries = %d", n)
	}
	// Entries replay.
	r.restart()
	if got := r.c.PROwners(ledger.Key("owner/repo", 13)); !reflect.DeepEqual(got, []string{key}) {
		t.Fatalf("owners after restart = %v", got)
	}
}

func TestDecisions(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	writeFile(t, filepath.Join(dir, "a.go"), "one\ntwo\nthree\nfour\n")
	by := decision.Party{Key: device + "/" + sidA, Name: "alpha", Kind: "claude"}
	d, err := r.c.RecordDecision(r.ctx, DecisionInput{By: by, Statement: " Keep two and three ", Why: "they pair", Path: "a.go", Lines: "2-3", Cwd: dir})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(d.ID, "d-") || d.Statement != "Keep two and three" || d.Anchor == nil ||
		d.Anchor.Path != "a.go" || d.Anchor.Start != 2 || d.Anchor.SHA != head(t, dir) || d.Trust != "untrusted" {
		t.Fatalf("decision = %+v", d)
	}
	list := r.c.Decisions(decision.Filter{})
	if len(list) != 1 || list[0].Status == nil || list[0].Status.State != decision.OK {
		t.Fatalf("decisions = %+v", list)
	}
	writeFile(t, filepath.Join(dir, "a.go"), "zero\none\ntwo\nthree\nfour\n")
	if s := r.c.Decisions(decision.Filter{})[0].Status; s.State != decision.Moved || s.Start != 3 {
		t.Fatalf("status = %+v", s)
	}
	why, err := r.c.Why(r.ctx, WhyQuery{Path: "a.go", Line: 3, Cwd: dir})
	if err != nil || len(why.Decisions) != 1 {
		t.Fatalf("why decisions = %+v, %v", why.Decisions, err)
	}
	if why, _ := r.c.Why(r.ctx, WhyQuery{Path: "a.go", Line: 1, Cwd: dir}); len(why.Decisions) != 0 {
		t.Fatalf("line 1 shows %d decisions", len(why.Decisions))
	}

	// Only the author or the person supersedes.
	other := decision.Party{Key: device + "/" + sidB, Kind: "claude"}
	if _, err := r.c.RecordDecision(r.ctx, DecisionInput{By: other, Statement: "no", Supersedes: []string{d.ID}}); err == nil {
		t.Fatal("another agent superseded")
	}
	user := decision.Party{Key: "user:" + device, Kind: "user"}
	d2, err := r.c.RecordDecision(r.ctx, DecisionInput{By: user, Trust: "user", Statement: "replaced", Supersedes: []string{d.ID}})
	if err != nil {
		t.Fatal(err)
	}
	if list := r.c.Decisions(decision.Filter{}); len(list) != 1 || list[0].ID != d2.ID || list[0].Status != nil {
		t.Fatalf("after supersede = %+v", list)
	}
	if list := r.c.Decisions(decision.Filter{All: true}); len(list) != 2 {
		t.Fatalf("all = %d", len(list))
	}
	if _, err := r.c.RecordDecision(r.ctx, DecisionInput{By: by, Statement: ""}); err != decision.ErrEmpty {
		t.Fatalf("empty: %v", err)
	}

	// The recorded root is gone: a live card's root in the same repo is used,
	// else the status is unknown.
	wt := filepath.Join(canon(t, t.TempDir()), "wt")
	gitCmd(t, dir, "worktree", "add", "-q", "-b", "side", wt)
	writeFile(t, filepath.Join(wt, "b.go"), "x\ny\n")
	db, err := r.c.RecordDecision(r.ctx, DecisionInput{By: by, Statement: "b", Path: filepath.Join(wt, "b.go")})
	if err != nil {
		t.Fatal(err)
	}
	gitCmd(t, dir, "worktree", "remove", "--force", wt)
	status := func() decision.AnchorState {
		for _, v := range r.c.Decisions(decision.Filter{}) {
			if v.ID == db.ID {
				return v.Status.State
			}
		}
		t.Fatal("missing")
		return ""
	}
	if s := status(); s != decision.Unknown {
		t.Fatalf("no worktree: %s", s)
	}
	r.roster.set(liveCard(sidA, dir, "idle"))
	if s := status(); s != decision.Gone {
		t.Fatalf("live card's root (b.go is not on main): %s", s)
	}
	if n := len(r.events(KindDecision)); n != 3 {
		t.Fatalf("decision rows = %d", n)
	}
	r.restart()
	if list := r.c.Decisions(decision.Filter{All: true}); len(list) != 3 {
		t.Fatalf("after restart = %d", len(list))
	}
}

func TestScrubbedSecretsNeverStored(t *testing.T) {
	const exact = "irori-lockfile-token-0123456789"
	ghToken := "ghp_" + strings.Repeat("A1b2", 9)
	r := newRig(t, func(o *Options) { o.Scrub.SetExact(exact) })
	dir := newRepo(t, "false")
	w := r.session(sidA, dir)
	now := time.Now()
	content := "package cfg\n\nconst key = \"" + ghToken + "\"\nconst tok = \"" + exact + "\"\n"
	w.write(flat(
		[]obj{w.prompt("p1", "use token "+ghToken+" and "+exact, now)},
		w2(w, "w1", filepath.Join(dir, "cfg.go"), content, now),
		w2(w, "w2", filepath.Join(dir, ".env"), "TOKEN="+ghToken+"\n", now),
		w.shell("b1", "Bash", "git commit -m x # token="+exact, "", nil, now),
		[]obj{w.reply("set "+ghToken, now), w.done(now)},
	)...)
	r.tail()

	evs, err := r.st.List(r.ctx, storeAll())
	if err != nil {
		t.Fatal(err)
	}
	for _, ev := range evs {
		if s := string(ev.Payload); strings.Contains(s, ghToken) || strings.Contains(s, exact) {
			t.Fatalf("%s row holds a secret: %s", ev.Kind, s)
		}
	}
	es := r.c.Edits(EditQuery{})
	if len(es) != 2 {
		t.Fatalf("edits = %d", len(es))
	}
	for _, e := range es {
		if e.Path == ".env" {
			if e.Patch != "" || e.Adds != 1 {
				t.Fatalf(".env edit = %+v", e)
			}
			continue
		}
		p, err := r.c.Blob(e.Patch)
		if err != nil || strings.Contains(string(p), ghToken) || strings.Contains(string(p), exact) || !strings.Contains(string(p), "[redacted:") {
			t.Fatalf("patch = %q, %v", p, err)
		}
	}
	tr := decode[TurnRow](t, r.rows(KindTurn)[0])
	if !strings.Contains(tr.Prompt, "[redacted:") || !strings.Contains(tr.Reply, "[redacted:") {
		t.Fatalf("turn = %+v", tr)
	}
}

func TestQuietClose(t *testing.T) {
	r := newRig(t, func(o *Options) { o.QuietAfter = time.Minute })
	cwd := canon(t, t.TempDir())
	w := r.session(sidA, cwd)
	r.roster.set(liveCard(sidA, cwd, "busy"))
	now := time.Now()
	w.write(w.prompt("p1", "long job", now), w.reply("working", now))
	r.tail()
	key := device + "/" + sidA
	tl := r.c.Timeline(TimelineQuery{Key: key})
	if len(tl.Entries) != 1 || !tl.Entries[0].Turn.Open || tl.Entries[0].Turn.Prompt != "long job" {
		t.Fatalf("open turn = %+v", tl.Entries)
	}
	r.clock.advance(2 * time.Minute)
	r.tail()
	if n := len(r.rows(KindTurn)); n != 0 {
		t.Fatal("closed a busy session's turn")
	}
	r.roster.set(liveCard(sidA, cwd, "idle"))
	r.tail()
	turns := r.rows(KindTurn)
	if len(turns) != 1 || decode[TurnRow](t, turns[0]).EndReason != "quiet" {
		t.Fatalf("turns = %d", len(turns))
	}
	if tl := r.c.Timeline(TimelineQuery{Key: key}); len(tl.Entries) != 1 || tl.Entries[0].Turn.Open {
		t.Fatalf("after close = %+v", tl.Entries)
	}
}

func TestTimelinePaging(t *testing.T) {
	r := newRig(t, nil)
	cwd := canon(t, t.TempDir())
	w := r.session(sidA, cwd)
	base := time.Now().Add(-time.Hour)
	for i := 0; i < 5; i++ {
		at := base.Add(time.Duration(i) * time.Minute)
		w.write(w.prompt("p"+itoa(i), "n", at), w.done(at.Add(time.Second)))
	}
	w.write(w.common("ai-title", time.Time{}, obj{"aiTitle": "A title"}))
	r.tail()
	key := device + "/" + sidA
	page := r.c.Timeline(TimelineQuery{Key: key, Limit: 2})
	ids := func(tl Timeline) []string {
		var out []string
		for _, e := range tl.Entries {
			if e.Turn != nil {
				out = append(out, e.Turn.Turn.ID)
			} else {
				out = append(out, string(e.Kind))
			}
		}
		return out
	}
	if got := ids(page); !reflect.DeepEqual(got, []string{"summary", "p4", "p3"}) || !page.More {
		t.Fatalf("page 1 = %v more=%v", got, page.More)
	}
	page = r.c.Timeline(TimelineQuery{Key: key, Limit: 2, Before: page.Entries[len(page.Entries)-1].At})
	if got := ids(page); !reflect.DeepEqual(got, []string{"p2", "p1"}) || !page.More {
		t.Fatalf("page 2 = %v", got)
	}
	page = r.c.Timeline(TimelineQuery{Key: key, Limit: 2, Before: page.Entries[len(page.Entries)-1].At})
	if got := ids(page); !reflect.DeepEqual(got, []string{"p0"}) || page.More {
		t.Fatalf("page 3 = %v", got)
	}
}

func TestRelinkLateEdits(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	r.roster.set(liveCard(sidB, dir, "idle"))
	r.repos() // watching dir
	foo := filepath.Join(dir, "foo.go")
	writeFile(t, foo, "package foo\n")
	at := time.Now()
	gitCmd(t, dir, "add", "foo.go")
	gitCmd(t, dir, "commit", "-q", "-m", "x")
	sha := head(t, dir)
	r.repos()
	if cv, _ := r.c.Commit(sha); cv.Link == nil || cv.Link.Files[0].Link != gitlink.LinkUnattributed {
		t.Fatalf("before = %+v", cv.Link)
	}
	// The transcript catches up after the commit was seen.
	w := r.session(sidA, dir)
	w.write(flat([]obj{w.prompt("p1", "x", at.Add(-time.Second))}, w2(w, "w1", foo, "package foo\n", at.Add(-time.Second)), []obj{w.done(at)})...)
	r.tail()
	cv, _ := r.c.Commit(sha)
	if cv.Link == nil || cv.Link.Files[0].Link != gitlink.LinkExact || !reflect.DeepEqual(cv.Link.Sessions, []string{device + "/" + sidA}) {
		t.Fatalf("after = %+v", cv.Link)
	}
	if n := len(r.rows(KindCommitLink)); n != 2 {
		t.Fatalf("links = %d", n)
	}
}

func TestRunAndSubscribe(t *testing.T) {
	r := newRig(t, nil)
	cwd := canon(t, t.TempDir())
	ctx, cancel := context.WithCancel(r.ctx)
	defer cancel()
	changes := r.c.Subscribe(ctx)
	done := make(chan error, 1)
	go func() { done <- r.c.Run(ctx) }()
	w := r.session(sidA, cwd)
	now := time.Now()
	w.write(w.prompt("p1", "hi", now), w.done(now))
	deadline := time.After(10 * time.Second)
	for {
		select {
		case ch := <-changes:
			if ch.Kind == ChangedTurn && ch.Key == device+"/"+sidA {
				cancel()
				if err := <-done; err != nil {
					t.Fatal(err)
				}
				if _, err := os.Stat(filepath.Join(r.data, cursorFileName)); err != nil {
					t.Fatal(err)
				}
				return
			}
		case <-deadline:
			t.Fatal("no turn change")
		}
	}
}

// The live order on a real machine: the edit is made and committed before
// the tailer has read the edit, so the root is first seen after its commit.
// First sight must still pick that commit up and link it exactly.
func TestCommitBeforeRootFirstSeen(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	w := r.session(sidA, dir)
	foo := filepath.Join(dir, "foo.go")
	content := "package foo\n"
	now := time.Now().Add(-5 * time.Second)
	w.write(flat(
		[]obj{w.prompt("p1", "add foo", now)},
		w.writeTool("w1", foo, content, nil, now.Add(time.Second)),
	)...)
	writeFile(t, foo, content)
	gitCmd(t, dir, "add", "foo.go")
	gitCmd(t, dir, "commit", "-q", "-m", "add foo")
	sha := head(t, dir)
	r.tail()
	r.repos()
	cv, ok := r.c.Commit(sha)
	if !ok || cv.Link == nil || len(cv.Link.Files) != 1 || cv.Link.Files[0].Link != "exact" {
		t.Fatalf("commit %s = %+v, ok %v", sha, cv, ok)
	}
}

// A repo pass over its budget stops after a commit and the next pass carries
// on from the same reflog line, so tailing is never starved and nothing is
// lost or doubled.
func TestRepoPassBudget(t *testing.T) {
	old := repoBudget
	repoBudget = 0
	t.Cleanup(func() { repoBudget = old })
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	w := r.session(sidA, dir)
	foo := filepath.Join(dir, "foo.go")
	now := time.Now().Add(-5 * time.Second)
	w.write(flat(
		[]obj{w.prompt("p1", "add foo", now)},
		w.writeTool("w1", foo, "package foo\n", nil, now.Add(time.Second)),
	)...)
	writeFile(t, foo, "package foo\n")
	// One reflog movement (a fast-forward) bringing in three commits.
	gitCmd(t, dir, "checkout", "-q", "-b", "side")
	for i, body := range []string{"package foo\n", "package foo\n// b\n", "package foo\n// c\n"} {
		writeFile(t, foo, body)
		gitCmd(t, dir, "add", "foo.go")
		gitCmd(t, dir, "commit", "-q", "--allow-empty", "-m", "c"+itoa(i))
	}
	gitCmd(t, dir, "checkout", "-q", "main")
	r.tail()
	gitCmd(t, dir, "merge", "-q", "--ff-only", "side")
	for i := 0; i < 10; i++ {
		r.repos()
	}
	// The repo's initial commit may fall inside the first-sight window too.
	if n := len(r.rows(KindCommitSeen)); n < 3 {
		t.Fatalf("commit.seen rows = %d, want the 3 fast-forwarded commits", n)
	}
	seen := map[string]bool{}
	for _, raw := range r.rows(KindCommitSeen) {
		row := decode[CommitRow](t, raw)
		if seen[row.SHA] {
			t.Fatalf("commit %s logged twice", row.SHA)
		}
		seen[row.SHA] = true
	}
}

// A long command (`git commit && gh pr checks --watch`) writes its result
// minutes after the commit, so the commit is linked before the command is
// read. When the command arrives, the commit gets its committer.
func TestLateCommitCommandSetsCommitter(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	w := r.session(sidA, dir)
	foo := filepath.Join(dir, "foo.go")
	start := time.Now().Add(-5 * time.Second)
	cmd := "python - <<'EOF'\nprint(1)\nEOF\ngit add foo.go && git commit -m 'add foo' && gh pr checks x --watch"
	w.write(flat(
		[]obj{w.prompt("p1", "add foo", start)},
		w.writeTool("w1", foo, "package foo\n", nil, start.Add(time.Second)),
		[]obj{w.toolUse("b1", "Bash", obj{"command": cmd, "description": "commit and watch"}, start.Add(2*time.Second))},
	)...)
	gitCmd(t, dir, "add", "foo.go")
	gitCmd(t, dir, "commit", "-q", "-m", "add foo")
	sha := head(t, dir)
	r.tail()
	r.repos()
	cv, ok := r.c.Commit(sha)
	if !ok || cv.Link == nil || cv.Link.Committer != nil {
		t.Fatalf("before the result: %+v ok=%v (want linked, no committer yet)", cv.Link, ok)
	}
	// The watch ends three minutes later.
	w.write(w.result("b1", obj{"stdout": "all checks passed\n", "stderr": "", "interrupted": false, "isImage": false}, time.Now().Add(3*time.Minute)))
	r.tail()
	cv, _ = r.c.Commit(sha)
	if cv.Link == nil || cv.Link.Committer == nil || cv.Link.Committer.Key != device+"/"+sidA {
		t.Fatalf("after the result: committer = %+v", cv.Link)
	}
	if len(cv.Link.Files) != 1 || cv.Link.Files[0].Link != "exact" {
		t.Fatalf("relink lost the files: %+v", cv.Link.Files)
	}
}

// Two late commit commands for one commit, read in the same pass, give it
// one new commit.linked, and calls racing on the same command can't log the
// same link twice (seen live: two identical rows for one commit).
func TestLateCommitCommandsLinkOnce(t *testing.T) {
	r := newRig(t, nil)
	dir := newRepo(t, "false")
	w := r.session(sidA, dir)
	foo := filepath.Join(dir, "foo.go")
	start := time.Now().Add(-5 * time.Second)
	cmd := "git add foo.go && git commit -m 'add foo' && gh pr checks x --watch"
	w.write(flat(
		[]obj{w.prompt("p1", "add foo", start)},
		w.writeTool("w1", foo, "package foo\n", nil, start.Add(time.Second)),
		[]obj{w.toolUse("b1", "Bash", obj{"command": cmd, "description": "commit and watch"}, start.Add(2*time.Second))},
		[]obj{w.toolUse("b2", "Bash", obj{"command": cmd, "description": "commit and watch"}, start.Add(2*time.Second))},
	)...)
	gitCmd(t, dir, "add", "foo.go")
	gitCmd(t, dir, "commit", "-q", "-m", "add foo")
	sha := head(t, dir)
	r.tail()
	r.repos()
	links := func() []CommitLink {
		var out []CommitLink
		for _, raw := range r.rows(KindCommitLink) {
			if l := decode[CommitLink](t, raw); l.SHA == sha {
				out = append(out, l)
			}
		}
		return out
	}
	first := links()
	if len(first) != 1 || first[0].Committer != nil {
		t.Fatalf("before the results: %+v", first)
	}
	done := time.Now().Add(3 * time.Minute)
	res := obj{"stdout": "ok\n", "stderr": "", "interrupted": false, "isImage": false}
	w.write(w.result("b1", res, done), w.result("b2", res, done))
	r.tail()
	if got := links(); len(got) != 2 || got[1].Committer == nil {
		t.Fatalf("after both results: %d links %+v", len(got), got)
	}

	// Race two calls on a commit that has lost its committer again.
	c := r.c.impl
	c.mu.Lock()
	rec := c.commitMap[commitKey(first[0].RepoKey, sha)]
	before := first[0]
	rec.link = &before
	cmds := append([]CommitCmdRow(nil), c.cmds...)
	c.mu.Unlock()
	if len(cmds) < 2 {
		t.Fatalf("commands = %d", len(cmds))
	}
	var wg sync.WaitGroup
	for _, cm := range cmds[:2] {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if err := c.relinkCommitter(r.ctx, cm); err != nil {
				t.Error(err)
			}
		}()
	}
	wg.Wait()
	if got := links(); len(got) != 3 {
		t.Fatalf("racing calls: %d links, want 3", len(got))
	}
	// Once it has a committer, another late command adds nothing.
	if err := c.relinkCommitter(r.ctx, cmds[0]); err != nil {
		t.Fatal(err)
	}
	if got := links(); len(got) != 3 {
		t.Fatalf("repeat: %d links, want 3", len(got))
	}
}

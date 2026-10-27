package presence

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Synthetic rows shaped like Claude Code's: a "permission-mode" row has only
// type, permissionMode and sessionId; user rows may carry permissionMode too.
func modeRow(mode string) string {
	return fmt.Sprintf(`{"type":"permission-mode","permissionMode":%q,"sessionId":"s"}`+"\n", mode)
}

func assistantRow(ts, text string) string {
	return fmt.Sprintf(`{"type":"assistant","timestamp":%q,"message":{"content":[{"type":"text","text":%q}]}}`+"\n", ts, text)
}

func TestReadTailPermissionMode(t *testing.T) {
	for name, tc := range map[string]struct{ src, want string }{
		"none":         {assistantRow("2026-10-26T10:00:00Z", "hi"), ""},
		"newest wins":  {modeRow("default") + assistantRow("2026-10-26T10:00:00Z", "hi") + modeRow("bypassPermissions"), "bypassPermissions"},
		"back to plan": {modeRow("bypassPermissions") + modeRow("plan"), "plan"},
		"user row": {modeRow("default") +
			`{"type":"user","timestamp":"2026-10-26T10:00:00Z","permissionMode":"acceptEdits","message":{"content":"x"}}` + "\n", "acceptEdits"},
		"sidechain ignored": {modeRow("default") +
			`{"type":"user","isSidechain":true,"permissionMode":"bypassPermissions","message":{"content":"x"}}` + "\n", "default"},
		"other types ignored": {modeRow("default") +
			`{"type":"system","permissionMode":"bypassPermissions"}` + "\n", "default"},
		"empty ignored": {modeRow("default") + modeRow(""), "default"},
	} {
		got := readTail(strings.NewReader(tc.src), int64(len(tc.src)), 1<<20)
		if got.PermissionMode != tc.want {
			t.Errorf("%s: PermissionMode = %q, want %q", name, got.PermissionMode, tc.want)
		}
	}
}

func appendFile(t *testing.T, p, s string) {
	t.Helper()
	f, err := os.OpenFile(p, os.O_APPEND|os.O_WRONLY|os.O_CREATE, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := f.WriteString(s); err != nil {
		t.Fatal(err)
	}
	f.Close()
}

func drain(ch <-chan Change) []Change {
	var out []Change
	for {
		select {
		case c := <-ch:
			out = append(out, c)
		default:
			return out
		}
	}
}

// A session that starts in default mode and switches to bypassPermissions:
// the switch is an Updated, and the mode survives once its row scrolls out.
func TestPermissionModeChangeEmitsUpdated(t *testing.T) {
	dir := t.TempDir()
	cwd := `C:\work\modes`
	id := "0b1c2d3e-0000-4000-8000-0000000000aa"
	writeSession(t, dir, map[string]any{"pid": 42, "sessionId": id, "cwd": cwd, "status": "busy"})
	tp := filepath.Join(dir, "projects", Slug(cwd), id+".jsonl")
	if err := os.MkdirAll(filepath.Dir(tp), 0o700); err != nil {
		t.Fatal(err)
	}
	appendFile(t, tp, modeRow("default")+assistantRow("2026-10-26T10:00:00Z", "one"))

	w, _ := testWatcher(dir, true)
	w.impl.tailBytes = 400
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	ch := w.Subscribe(ctx)

	w.impl.scan()
	if cs := drain(ch); len(cs) != 1 || cs[0].Kind != Added || cs[0].Card.PermissionMode != "default" {
		t.Fatalf("first scan: %+v", cs)
	}

	// The mode switch alone: the row has no timestamp, nothing else moves.
	appendFile(t, tp, modeRow("bypassPermissions"))
	w.impl.scan()
	cs := drain(ch)
	if len(cs) != 1 || cs[0].Kind != Updated || cs[0].Card.PermissionMode != "bypassPermissions" {
		t.Fatalf("after switch: %+v", cs)
	}
	if cs[0].Card.LastText != "one" {
		t.Errorf("switch lost other fields: %+v", cs[0].Card)
	}

	// Push the mode row out of the 400-byte window; the cached mode stays.
	for i := range 6 {
		appendFile(t, tp, assistantRow(fmt.Sprintf("2026-10-26T10:0%d:00Z", i+1), strings.Repeat("x", 80)))
	}
	raw, err := os.ReadFile(tp)
	if err != nil {
		t.Fatal(err)
	}
	if got := readTail(strings.NewReader(string(raw)), int64(len(raw)), 400); got.PermissionMode != "" {
		t.Fatalf("test setup: window still holds a mode row (%q)", got.PermissionMode)
	}
	w.impl.scan()
	cards := w.Snapshot()
	if len(cards) != 1 || cards[0].PermissionMode != "bypassPermissions" {
		t.Fatalf("mode lost after scrolling out: %+v", cards)
	}
	for _, c := range drain(ch) {
		if c.Card.PermissionMode != "bypassPermissions" {
			t.Errorf("change with lost mode: %+v", c)
		}
	}
}

// RepoKey is resolved once per cwd, not on every scan or for every session.
func TestRepoKeyResolvedOncePerCwd(t *testing.T) {
	dir := t.TempDir()
	calls := map[string]int{} // scans run on this goroutine only
	resolve := func(cwd string) string {
		calls[cwd]++
		if filepath.Base(cwd) == "outside" {
			return ""
		}
		return "key:" + filepath.Base(cwd)
	}
	repoA := filepath.Join(dir, "a")
	outside := filepath.Join(dir, "outside")
	writeSession(t, dir, map[string]any{"pid": 1, "sessionId": "s1", "cwd": repoA})
	writeSession(t, dir, map[string]any{"pid": 2, "sessionId": "s2", "cwd": repoA})
	writeSession(t, dir, map[string]any{"pid": 3, "sessionId": "s3", "cwd": outside})

	w, _ := testWatcher(dir, true)
	w.impl.opts.RepoKey = resolve
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	ch := w.Subscribe(ctx)
	for range 3 {
		w.impl.scan()
	}
	if calls[repoA] != 1 || calls[outside] != 1 {
		t.Fatalf("resolver calls: %v", calls)
	}
	byID := map[string]AgentCard{}
	for _, c := range w.Snapshot() {
		byID[c.SessionID] = c
	}
	if byID["s1"].RepoKey != "key:a" || byID["s2"].RepoKey != "key:a" || byID["s3"].RepoKey != "" {
		t.Fatalf("repo keys: %+v", byID)
	}
	drain(ch)

	// s3 moves into a repo: new cwd, new key, one Updated.
	repoB := filepath.Join(dir, "b")
	writeSession(t, dir, map[string]any{"pid": 3, "sessionId": "s3", "cwd": repoB})
	w.impl.scan()
	cs := drain(ch)
	if len(cs) != 1 || cs[0].Kind != Updated || cs[0].Card.RepoKey != "key:b" {
		t.Fatalf("cwd move: %+v", cs)
	}
	if calls[repoB] != 1 {
		t.Fatalf("resolver calls: %v", calls)
	}
	// A cwd no live session uses any more is forgotten.
	if _, ok := w.impl.repos[outside]; ok {
		t.Error("cache kept a cwd with no live session")
	}
}

func TestRepoKeyNilResolver(t *testing.T) {
	dir := t.TempDir()
	writeSession(t, dir, map[string]any{"pid": 1, "sessionId": "s1", "cwd": dir})
	w, _ := testWatcher(dir, true)
	w.impl.scan()
	if cards := w.Snapshot(); len(cards) != 1 || cards[0].RepoKey != "" {
		t.Fatalf("cards: %+v", cards)
	}
}

func TestChangedSeesV02Fields(t *testing.T) {
	base := AgentCard{Key: "d/s"}
	b := base
	b.RepoKey = "k"
	if !changed(base, b) {
		t.Error("repoKey change ignored")
	}
	b = base
	b.PermissionMode = "bypassPermissions"
	if !changed(base, b) {
		t.Error("permissionMode change ignored")
	}
}

// Cold start: a fresh watcher whose tail window holds no mode row still finds
// the newest one further back, and records that it looked.
func TestColdStartFindsModeBeyondTail(t *testing.T) {
	dir := t.TempDir()
	cwd := `C:\work\cold`
	id := "0b1c2d3e-0000-4000-8000-0000000000bb"
	writeSession(t, dir, map[string]any{"pid": 42, "sessionId": id, "cwd": cwd, "status": "busy"})
	tp := filepath.Join(dir, "projects", Slug(cwd), id+".jsonl")
	if err := os.MkdirAll(filepath.Dir(tp), 0o700); err != nil {
		t.Fatal(err)
	}
	appendFile(t, tp, modeRow("default")+
		`{"type":"user","timestamp":"2026-10-26T09:59:00Z","permissionMode":"bypassPermissions","message":{"content":"go"}}`+"\n")
	for i := range 6 {
		appendFile(t, tp, assistantRow(fmt.Sprintf("2026-10-26T10:0%d:00Z", i), strings.Repeat("x", 80)))
	}
	raw, err := os.ReadFile(tp)
	if err != nil {
		t.Fatal(err)
	}
	if got := readTail(strings.NewReader(string(raw)), int64(len(raw)), 400); got.PermissionMode != "" {
		t.Fatalf("test setup: window still holds a mode row (%q)", got.PermissionMode)
	}

	w, _ := testWatcher(dir, true)
	w.impl.tailBytes = 400
	w.impl.scan()
	if cards := w.Snapshot(); len(cards) != 1 || cards[0].PermissionMode != "bypassPermissions" {
		t.Fatalf("cold start mode: %+v", cards)
	}
	if tc := w.impl.tails[id]; tc == nil || !tc.modeScanned {
		t.Fatalf("scan not recorded: %+v", tc)
	}
}

// The backward scan is bounded and skips the partial first line.
func TestScanMode(t *testing.T) {
	far := modeRow("acceptEdits")
	pad := assistantRow("2026-10-26T10:00:00Z", strings.Repeat("y", 200))
	body := far + pad
	if got := scanMode(strings.NewReader(body), int64(len(body)), int64(len(body))); got != "acceptEdits" {
		t.Errorf("whole file: %q", got)
	}
	if got := scanMode(strings.NewReader(body), int64(len(body)), int64(len(pad))); got != "" {
		t.Errorf("row outside bound found: %q", got)
	}
	// A window opening exactly on the mode row keeps it.
	edge := pad + body
	if got := scanMode(strings.NewReader(edge), int64(len(edge)), int64(len(body))); got != "acceptEdits" {
		t.Errorf("row on boundary: %q", got)
	}
	cut := pad + far[3:] + pad
	if got := scanMode(strings.NewReader(cut), int64(len(cut)), int64(len(cut)-len(pad)-2)); got != "" {
		t.Errorf("partial line parsed: %q", got)
	}
	newest := modeRow("default") + far + `{"type":"user","isSidechain":true,"permissionMode":"bypassPermissions"}` + "\n" + pad
	if got := scanMode(strings.NewReader(newest), int64(len(newest)), 1<<20); got != "acceptEdits" {
		t.Errorf("newest non-sidechain: %q", got)
	}
}

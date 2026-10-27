package presence

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

const (
	fixtureID  = "f749a456-093f-4fa1-a63a-da64e6d59e95"
	fixtureID2 = "27e38ea9-dd8c-45b0-aa02-602bccf1f0ad"
)

// opens records every path the watcher opened.
type opens struct {
	mu    sync.Mutex
	paths []string
}

func (o *opens) open(name string) (*os.File, error) {
	o.mu.Lock()
	o.paths = append(o.paths, name)
	o.mu.Unlock()
	return os.Open(name)
}

func (o *opens) count(suffix string) int {
	o.mu.Lock()
	defer o.mu.Unlock()
	n := 0
	for _, p := range o.paths {
		if strings.HasSuffix(p, suffix) {
			n++
		}
	}
	return n
}

func testWatcher(dir string, allLive bool) (*Watcher, *opens) {
	w := New(Options{ClaudeDir: dir, DeviceID: "dev1", Interval: 10 * time.Millisecond})
	o := &opens{}
	w.impl.open = o.open
	if allLive {
		w.impl.live = func(int, uint64) bool { return true }
	}
	return w, o
}

// writeSession writes a synthetic <pid>.json into dir/sessions.
func writeSession(t *testing.T, dir string, fields map[string]any) string {
	t.Helper()
	if err := os.MkdirAll(filepath.Join(dir, "sessions"), 0o700); err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(fields)
	p := filepath.Join(dir, "sessions", fmt.Sprintf("%v.json", fields["pid"]))
	if err := os.WriteFile(p, raw, 0o600); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestSlug(t *testing.T) {
	for in, want := range map[string]string{
		`C:\Users\4step`:                      "C--Users-4step",
		"/Users/stephenhorton/projects/irori": "-Users-stephenhorton-projects-irori",
		`C:\Users\4step\projects\irori`:       "C--Users-4step-projects-irori",
		"/home/me/my_app.v2":                  "-home-me-my-app-v2",
		"/tmp/café":                           "-tmp-caf-",
		"":                                    "",
	} {
		if got := Slug(in); got != want {
			t.Errorf("Slug(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestIsSessionFile(t *testing.T) {
	for name, want := range map[string]bool{
		"13704.json": true,
		"39668.0000000000000000000000000000000000000000000000000000000000000000.key": false,
		"39668.key":      false,
		"39668.abc.json": false,
		".json":          false,
		"x1.json":        false,
		"123.json.bak":   false,
	} {
		if got := isSessionFile(name); got != want {
			t.Errorf("isSessionFile(%q) = %v", name, got)
		}
	}
}

func TestReadSessionFixtures(t *testing.T) {
	f, err := os.Open(filepath.Join("testdata", "sessions", "13704.json"))
	if err != nil {
		t.Fatal(err)
	}
	s, ok := readSession(f)
	f.Close()
	if !ok || s.PID != 13704 || s.ID != fixtureID || s.Cwd != `C:\Users\4step` {
		t.Fatalf("13704: %+v", s)
	}
	if s.ProcStart != 134375324500655525 || s.Status != "busy" || s.Kind != "interactive" || len(s.PeerFeatures) != 2 {
		t.Fatalf("13704 fields: %+v", s)
	}
	if !s.UpdatedAt.Equal(time.UnixMilli(1793063865535)) || !s.StartedAt.Equal(time.UnixMilli(1793058851218)) {
		t.Fatalf("13704 times: %v %v", s.StartedAt, s.UpdatedAt)
	}

	// 39668.json is a bg session with an escaped Windows cwd.
	f, err = os.Open(filepath.Join("testdata", "sessions", "39668.json"))
	if err != nil {
		t.Fatal(err)
	}
	s, ok = readSession(f)
	f.Close()
	if !ok || s.PID != 39668 || s.Cwd != `C:\Users\4step` || s.Kind != "bg" || s.Name != "hello-spike" {
		t.Fatalf("39668: ok=%v %+v", ok, s)
	}

	for _, bad := range []string{``, `{`, `{"pid":0,"sessionId":"x"}`, `{"pid":5}`, `[1,2]`} {
		if _, ok := readSession(strings.NewReader(bad)); ok {
			t.Errorf("readSession(%q) accepted", bad)
		}
	}
	// Unknown fields and odd types are tolerated.
	s, ok = readSession(strings.NewReader(`{"pid":"7","sessionId":"abc","procStart":12,"peerFeatures":[1,"x"],"new":{"a":1}}`))
	if !ok || s.PID != 7 || s.ProcStart != 12 || len(s.PeerFeatures) != 1 {
		t.Fatalf("tolerant: ok=%v %+v", ok, s)
	}
}

func TestReadTailSkipsPartialLine(t *testing.T) {
	p := filepath.Join("testdata", "projects", "C--Users-4step", fixtureID+".jsonl")
	raw, err := os.ReadFile(p)
	if err != nil {
		t.Fatal(err)
	}
	pad := bytes.Index(raw, []byte("PADDING"))
	if pad < 0 {
		t.Fatal("fixture lost its padding row")
	}
	size := int64(len(raw))
	n := size - int64(pad+100) // window opens inside the padding row

	got := readTail(bytes.NewReader(raw), size, n)
	if got.Title != "Port the roster" {
		t.Errorf("Title = %q", got.Title)
	}
	if got.LastText != "Roster card shows the last reply." {
		t.Errorf("LastText = %q", got.LastText)
	}
	if want := time.Date(2026, 10, 26, 10, 3, 0, 0, time.UTC); !got.LastActivity.Equal(want) {
		t.Errorf("LastActivity = %v", got.LastActivity)
	}

	// The partial row's timestamp (10:00:05) and everything before it stay out.
	got = readTail(bytes.NewReader(raw[:pad+200]), int64(pad+200), 150)
	if !got.LastActivity.IsZero() || got.Title != "" || got.LastText != "" {
		t.Errorf("partial-only window read something: %+v", got)
	}

	// A window opening exactly on a row keeps it; one byte later drops it.
	row := bytes.Index(raw, []byte(`{"type":"custom-title","customTitle":"Port the roster"`))
	if row < 0 {
		t.Fatal("fixture lost its in-window custom title")
	}
	if got = readTail(bytes.NewReader(raw), size, size-int64(row)); got.Title != "Port the roster" {
		t.Errorf("aligned window: Title = %q", got.Title)
	}
	if got = readTail(bytes.NewReader(raw), size, size-int64(row)-1); got.Title != "Synthetic ai title" {
		t.Errorf("window one byte into the row: Title = %q", got.Title)
	}

	// The whole file: the newest custom title still wins over the ai title.
	got = readTail(bytes.NewReader(raw), size, size+10)
	if got.Title != "Port the roster" || got.LastText != "Roster card shows the last reply." {
		t.Errorf("whole file: %+v", got)
	}
}

func TestTitleFallsBackToAITitle(t *testing.T) {
	src := `{"type":"ai-title","aiTitle":"first"}` + "\n" +
		`{"type":"ai-title","aiTitle":"second"}` + "\n"
	got := readTail(strings.NewReader(src), int64(len(src)), 1<<20)
	if got.Title != "second" {
		t.Fatalf("Title = %q", got.Title)
	}
}

func TestOneLine(t *testing.T) {
	if got := oneLine(" a\n\n b\t c ", 240); got != "a b c" {
		t.Errorf("collapse: %q", got)
	}
	long := strings.Repeat("é", 500)
	got := oneLine(long, maxLastText)
	if n := len([]rune(got)); n != maxLastText || !strings.HasSuffix(got, "…") {
		t.Errorf("cap: %d runes", n)
	}
}

// The <pid>.<hash>.key decoy in testdata must never be opened.
func TestScanNeverOpensKeyFiles(t *testing.T) {
	w, o := testWatcher("testdata", true)
	w.impl.scan()
	o.mu.Lock()
	for _, p := range o.paths {
		if strings.HasSuffix(p, ".key") {
			t.Errorf("opened secret %s", p)
		}
	}
	o.mu.Unlock()
	if o.count(".json") != 2 {
		t.Errorf("want both session files read, got %v", o.paths)
	}

	cards := w.Snapshot()
	if len(cards) != 2 {
		t.Fatalf("want 2 cards, got %d", len(cards))
	}
	a, b := cards[0], cards[1]
	if a.SessionID != fixtureID || b.SessionID != fixtureID2 {
		t.Fatalf("order: %s, %s", a.SessionID, b.SessionID)
	}
	if a.Key != "dev1/"+fixtureID || a.Agent != "claude" || a.DeviceID != "dev1" || a.Name != "ww-multiplayer-oct-26" {
		t.Errorf("card a: %+v", a)
	}
	if a.Title != "Port the roster" || a.LastText != "Roster card shows the last reply." {
		t.Errorf("card a tail: %+v", a)
	}
	if a.PermissionMode != "bypassPermissions" || b.PermissionMode != "" {
		t.Errorf("permission modes: a=%q b=%q", a.PermissionMode, b.PermissionMode)
	}
	if !filepath.IsAbs(a.Transcript) || !strings.HasSuffix(a.Transcript, filepath.Join("C--Users-4step", fixtureID+".jsonl")) {
		t.Errorf("transcript a = %q", a.Transcript)
	}
	// Found through the projects/*/<id>.jsonl fallback.
	if !strings.HasSuffix(b.Transcript, filepath.Join("elsewhere", fixtureID2+".jsonl")) || b.Title != "Hello spike" {
		t.Errorf("card b: %+v", b)
	}
}

func TestDeadPIDNotListed(t *testing.T) {
	dir := t.TempDir()
	writeSession(t, dir, map[string]any{"pid": 1 << 30, "sessionId": "dead-session", "cwd": dir})
	writeSession(t, dir, map[string]any{"pid": os.Getpid(), "sessionId": "me-session", "cwd": dir})
	w, _ := testWatcher(dir, false)
	w.impl.scan()
	cards := w.Snapshot()
	if len(cards) != 1 || cards[0].SessionID != "me-session" {
		t.Fatalf("got %+v", cards)
	}
	if cards[0].Name != filepath.Base(dir) || cards[0].Transcript != "" {
		t.Errorf("derived name / missing transcript: %+v", cards[0])
	}
}

func TestTailReadOnlyWhenTranscriptMoves(t *testing.T) {
	dir := t.TempDir()
	cwd := `C:\work\demo`
	id := "0b1c2d3e-0000-4000-8000-000000000001"
	writeSession(t, dir, map[string]any{"pid": 42, "sessionId": id, "cwd": cwd, "status": "busy"})
	tp := filepath.Join(dir, "projects", Slug(cwd), id+".jsonl")
	if err := os.MkdirAll(filepath.Dir(tp), 0o700); err != nil {
		t.Fatal(err)
	}
	row := func(ts, text string) string {
		return fmt.Sprintf(`{"type":"assistant","timestamp":%q,"message":{"content":[{"type":"text","text":%q}]}}`+"\n", ts, text)
	}
	if err := os.WriteFile(tp, []byte(row("2026-10-26T10:00:00Z", "one")), 0o600); err != nil {
		t.Fatal(err)
	}

	w, o := testWatcher(dir, true)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	ch := w.Subscribe(ctx)
	w.impl.scan()
	w.impl.scan()
	w.impl.scan()
	if n := o.count(".jsonl"); n != 1 {
		t.Fatalf("transcript opened %d times for an unchanged file", n)
	}
	if c := <-ch; c.Kind != Added || c.Card.LastText != "one" {
		t.Fatalf("first change: %+v", c)
	}

	f, err := os.OpenFile(tp, os.O_APPEND|os.O_WRONLY, 0)
	if err != nil {
		t.Fatal(err)
	}
	f.WriteString(row("2026-10-26T10:00:05Z", "two"))
	f.Close()
	w.impl.scan()
	if n := o.count(".jsonl"); n != 2 {
		t.Fatalf("transcript opened %d times after it grew", n)
	}
	select {
	case c := <-ch:
		if c.Kind != Updated || c.Card.LastText != "two" {
			t.Fatalf("second change: %+v", c)
		}
	default:
		t.Fatal("no Updated after the transcript grew")
	}
}

func TestChangedIgnoresSubSecondActivity(t *testing.T) {
	base := AgentCard{Key: "d/s", PeerFeatures: []string{"a"}, LastActivity: time.Unix(100, 0)}
	b := base
	b.LastActivity = base.LastActivity.Add(500 * time.Millisecond)
	if changed(base, b) {
		t.Error("sub-second activity counted as a change")
	}
	b.LastActivity = base.LastActivity.Add(time.Second)
	if !changed(base, b) {
		t.Error("a full second of activity was ignored")
	}
	b = base
	b.PeerFeatures = []string{"a", "b"}
	if !changed(base, b) {
		t.Error("peerFeatures change ignored")
	}
	b = base
	b.Status = "idle"
	if !changed(base, b) {
		t.Error("status change ignored")
	}
}

func TestSubscribeAddedThenRemoved(t *testing.T) {
	dir := t.TempDir()
	p := writeSession(t, dir, map[string]any{"pid": os.Getpid(), "sessionId": "sub-session", "cwd": dir, "status": "idle"})
	w, _ := testWatcher(dir, false)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	ch := w.Subscribe(ctx)
	done := make(chan error, 1)
	go func() { done <- w.Run(ctx) }()

	next := func() Change {
		t.Helper()
		select {
		case c := <-ch:
			return c
		case <-time.After(5 * time.Second):
			t.Fatal("timed out waiting for a change")
			return Change{}
		}
	}
	if c := next(); c.Kind != Added || c.Card.SessionID != "sub-session" || c.Card.Status != "idle" {
		t.Fatalf("want Added, got %+v", c)
	}
	if err := os.Remove(p); err != nil {
		t.Fatal(err)
	}
	if c := next(); c.Kind != Removed || c.Card.Status != "gone" || c.Card.Key != "dev1/sub-session" {
		t.Fatalf("want Removed gone, got %+v", c)
	}
	if len(w.Snapshot()) != 0 {
		t.Error("snapshot still lists the removed session")
	}
	cancel()
	if err := <-done; err != nil {
		t.Fatal(err)
	}
	if _, open := <-ch; open {
		// Drain anything buffered; the channel must close after ctx ends.
		for range ch {
		}
	}
}

// Invalid session JSON is skipped, never guessed at: a repair would turn the
// "\t" in C:\Users\4step\tools into a TAB.
func TestReadSessionRejectsInvalidJSON(t *testing.T) {
	raw := `{"pid":5,"sessionId":"a","cwd":"C:` + "\\" + `Users` + "\\" + `4step` + "\\" + `tools"}`
	if _, ok := readSession(strings.NewReader(raw)); ok {
		t.Fatal("invalid JSON was accepted")
	}
}

package capture

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/repo"
	"github.com/StephenSHorton/irori/internal/scrub"
	"github.com/StephenSHorton/irori/internal/store"
)

// Everything here is synthetic: transcripts are built from the record
// shapes the transcript package documents, repos are fresh temp repos.

const device = "dev1"

// fakeRoster is a settable presence roster.
type fakeRoster struct {
	mu    sync.Mutex
	cards []presence.AgentCard
}

func (r *fakeRoster) Agents() []presence.AgentCard {
	r.mu.Lock()
	defer r.mu.Unlock()
	return append([]presence.AgentCard(nil), r.cards...)
}

func (r *fakeRoster) set(cards ...presence.AgentCard) {
	r.mu.Lock()
	r.cards = cards
	r.mu.Unlock()
}

func liveCard(sid, cwd, status string) presence.AgentCard {
	return presence.AgentCard{Key: device + "/" + sid, DeviceID: device, Agent: "claude", SessionID: sid, Cwd: cwd, Status: status}
}

// clock is a settable Now.
type clock struct {
	mu  sync.Mutex
	off time.Duration
}

func (c *clock) now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return time.Now().Add(c.off)
}

func (c *clock) advance(d time.Duration) {
	c.mu.Lock()
	c.off += d
	c.mu.Unlock()
}

// rig is one capture service over a temp data dir and a fake ~/.claude.
type rig struct {
	t      *testing.T
	ctx    context.Context
	data   string
	claude string
	st     *store.Store
	c      *Capture
	roster *fakeRoster
	clock  *clock
	opts   Options
}

func canon(t *testing.T, dir string) string {
	t.Helper()
	d, err := filepath.EvalSymlinks(dir)
	if err != nil {
		t.Fatal(err)
	}
	return d
}

func newRig(t *testing.T, mod func(*Options)) *rig {
	t.Helper()
	r := &rig{t: t, ctx: context.Background(), data: canon(t, t.TempDir()), claude: canon(t, t.TempDir()),
		roster: &fakeRoster{}, clock: &clock{}}
	r.opts = Options{
		ClaudeDir: r.claude, DataDir: r.data, DeviceID: device, Now: r.clock.now,
		RepoKey: repo.New().Key, Scrub: scrub.New(),
		ScanInterval: 50 * time.Millisecond, RepoInterval: 50 * time.Millisecond,
	}
	if mod != nil {
		mod(&r.opts)
	}
	r.open()
	return r
}

// open (re)opens the store and a fresh service over the same dirs.
func (r *rig) open() {
	r.t.Helper()
	st, err := store.Open(r.ctx, filepath.Join(r.data, "irori.db"), device, hlc.New(nil))
	if err != nil {
		r.t.Fatal(err)
	}
	c, err := New(r.ctx, st, r.roster, r.opts)
	if err != nil {
		st.Close()
		r.t.Fatal(err)
	}
	r.st, r.c = st, c
	r.t.Cleanup(func() { st.Close() })
}

// restart closes the store and opens everything again.
func (r *rig) restart() {
	r.t.Helper()
	r.st.Close()
	r.open()
}

func (r *rig) tail()  { r.t.Helper(); r.c.impl.tailPass(r.ctx) }
func (r *rig) repos() { r.t.Helper(); r.c.impl.repoPass(r.ctx) }

// rows lists the logged payloads of one kind.
func (r *rig) rows(kind string) []json.RawMessage {
	r.t.Helper()
	evs, err := r.st.List(r.ctx, store.Filter{Kinds: []string{kind}, Limit: store.MaxListLimit})
	if err != nil {
		r.t.Fatal(err)
	}
	out := make([]json.RawMessage, len(evs))
	for i, ev := range evs {
		out[i] = ev.Payload
	}
	return out
}

// counts maps every capture kind to its row count.
func (r *rig) counts() map[string]int {
	out := map[string]int{}
	for _, k := range replayKinds {
		out[k] = len(r.rows(k))
	}
	return out
}

func storeAll() store.Filter { return store.Filter{Limit: store.MaxListLimit} }

func (r *rig) events(kind string) []store.Event {
	r.t.Helper()
	evs, err := r.st.List(r.ctx, store.Filter{Kinds: []string{kind}, Limit: store.MaxListLimit})
	if err != nil {
		r.t.Fatal(err)
	}
	return evs
}

// --- git ---

func gitCmd(t *testing.T, dir string, args ...string) string {
	t.Helper()
	full := append([]string{"-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"}, args...)
	cmd := exec.Command("git", full...)
	cmd.Dir = dir
	var env []string
	for _, kv := range os.Environ() {
		if !strings.HasPrefix(strings.ToUpper(kv), "GIT_") {
			env = append(env, kv)
		}
	}
	cmd.Env = env
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
	return strings.TrimSpace(string(out))
}

// newRepo makes a repo with one commit and core.autocrlf set as given.
func newRepo(t *testing.T, autocrlf string) string {
	t.Helper()
	dir := canon(t, t.TempDir())
	gitCmd(t, dir, "init", "-q")
	gitCmd(t, dir, "config", "core.autocrlf", autocrlf)
	writeFile(t, filepath.Join(dir, "README.md"), "hello\n")
	gitCmd(t, dir, "add", "README.md")
	gitCmd(t, dir, "commit", "-q", "-m", "init")
	return dir
}

func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func head(t *testing.T, dir string) string { return gitCmd(t, dir, "rev-parse", "HEAD") }

// --- transcripts ---

type obj = map[string]any

// tw writes one synthetic transcript file.
type tw struct {
	t       *testing.T
	path    string
	sid     string
	cwd     string
	agentID string
	n       int
}

func (r *rig) session(sid, cwd string) *tw {
	slug := strings.NewReplacer(`\`, "-", "/", "-", ":", "-").Replace(cwd)
	return &tw{t: r.t, path: filepath.Join(r.claude, "projects", slug, sid+".jsonl"), sid: sid, cwd: cwd}
}

func (r *rig) subagent(sid, cwd, agentID string) *tw {
	w := r.session(sid, cwd)
	w.path = filepath.Join(strings.TrimSuffix(w.path, ".jsonl"), "subagents", "agent-"+agentID+".jsonl")
	w.agentID = agentID
	return w
}

func ts(at time.Time) string { return at.UTC().Format("2006-01-02T15:04:05.000Z") }

func (w *tw) uuid() string {
	w.n++
	return w.sid[:8] + "-u" + strings.Repeat("0", 3) + string(rune('a'+w.n%26)) + itoa(w.n)
}

func itoa(n int) string {
	b, _ := json.Marshal(n)
	return string(b)
}

func (w *tw) common(typ string, at time.Time, extra obj) obj {
	m := obj{
		"parentUuid": nil, "isSidechain": w.agentID != "", "type": typ, "uuid": w.uuid(),
		"timestamp": ts(at), "userType": "external", "entrypoint": "cli", "cwd": w.cwd,
		"sessionId": w.sid, "version": "2.1.300", "gitBranch": "main",
	}
	if w.agentID != "" {
		m["agentId"] = w.agentID
	}
	for k, v := range extra {
		m[k] = v
	}
	return m
}

func (w *tw) prompt(promptID, text string, at time.Time) obj {
	return w.common("user", at, obj{"promptId": promptID, "message": obj{"role": "user", "content": text}})
}

func (w *tw) toolUse(id, name string, input obj, at time.Time) obj {
	return w.common("assistant", at, obj{"message": obj{"id": "msg_" + id, "role": "assistant", "type": "message",
		"content": []any{obj{"type": "tool_use", "id": id, "name": name, "input": input}}}})
}

func (w *tw) reply(text string, at time.Time) obj {
	return w.common("assistant", at, obj{"message": obj{"id": "msg_r" + itoa(w.n), "role": "assistant", "type": "message",
		"content": []any{obj{"type": "text", "text": text}}}})
}

func (w *tw) result(toolID string, tur any, at time.Time) obj {
	return w.common("user", at, obj{
		"message": obj{"role": "user", "content": []any{
			obj{"type": "tool_result", "tool_use_id": toolID, "is_error": false, "content": "ok"},
		}},
		"toolUseResult": tur,
	})
}

func (w *tw) done(at time.Time) obj {
	return w.common("system", at, obj{"subtype": "turn_duration", "durationMs": 1000})
}

// writeTool is a Write call and its result, also writing the file to disk.
func (w *tw) writeTool(id, path, content string, original *string, at time.Time) []obj {
	w.t.Helper()
	tur := obj{"type": "create", "filePath": path, "content": content, "structuredPatch": []any{}, "originalFile": nil}
	if original != nil {
		tur["type"] = "update"
		tur["originalFile"] = *original
		tur["structuredPatch"] = []any{obj{"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 1,
			"lines": []string{"-" + firstLine(*original), "+" + firstLine(content)}}}
	}
	writeFile(w.t, path, content)
	return []obj{
		w.toolUse(id, "Write", obj{"file_path": path, "content": content}, at),
		w.result(id, tur, at.Add(100*time.Millisecond)),
	}
}

// shell is a Bash or PowerShell call and its result.
func (w *tw) shell(id, tool, command, stdout string, gitOp obj, at time.Time) []obj {
	tur := obj{"stdout": stdout, "stderr": "", "interrupted": false, "isImage": false}
	if gitOp != nil {
		tur["gitOperation"] = gitOp
	}
	return []obj{
		w.toolUse(id, tool, obj{"command": command, "description": "run"}, at),
		w.result(id, tur, at),
	}
}

func lineOf(t *testing.T, m obj) []byte {
	t.Helper()
	b, err := json.Marshal(m)
	if err != nil {
		t.Fatal(err)
	}
	return append(b, '\n')
}

// write appends rows to the file.
func (w *tw) write(rows ...obj) {
	w.t.Helper()
	var buf []byte
	for _, m := range rows {
		buf = append(buf, lineOf(w.t, m)...)
	}
	w.appendBytes(buf)
}

func (w *tw) appendBytes(b []byte) {
	w.t.Helper()
	if err := os.MkdirAll(filepath.Dir(w.path), 0o755); err != nil {
		w.t.Fatal(err)
	}
	f, err := os.OpenFile(w.path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
	if err != nil {
		w.t.Fatal(err)
	}
	defer f.Close()
	if _, err := f.Write(b); err != nil {
		w.t.Fatal(err)
	}
}

// writeInPieces appends rows split mid-line, tailing between the pieces,
// as a live writer would leave them.
func (w *tw) writeInPieces(r *rig, rows ...obj) {
	w.t.Helper()
	for _, m := range rows {
		b := lineOf(w.t, m)
		half := len(b) / 2
		w.appendBytes(b[:half])
		r.tail()
		w.appendBytes(b[half:])
	}
	r.tail()
}

func flat(groups ...[]obj) []obj {
	var out []obj
	for _, g := range groups {
		out = append(out, g...)
	}
	return out
}

func decode[T any](t *testing.T, raw json.RawMessage) T {
	t.Helper()
	var v T
	if err := json.Unmarshal(raw, &v); err != nil {
		t.Fatal(err)
	}
	return v
}

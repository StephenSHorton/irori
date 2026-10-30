// Golden exporter for crates/presence (mesh.md §3.4, surface.md §2.13).
//
// This file is not part of the Go repo. tools/goldens/export.sh injects it
// into I:internal/presence with `go test -overlay` (package presence, so it
// reaches readSession, readTail, scanMode, the watcher and the test
// helpers). It only runs when IRORI_GOLDEN_OUT is set, and writes
// $IRORI_GOLDEN_OUT/presence/presence.json:
//
//	sessions   readSession over JSON vectors (times in UTC)
//	tails      readTail over synthetic transcripts and (size, n) windows
//	modes      scanMode over the same
//	lstart     parseLstart
//	slug, oneLine, displayName, isSessionFile
//	json       json.Marshal of a zero AgentCard and Change, a full card, and
//	           the two fixture cards from a scan of crates/presence/testdata
//	           with every pid live ($TESTDATA for the fixture dir, times
//	           in UTC)
//	local      Go's local offset for instants across years, with the
//	           reference instant the Rust test compares first
//
// Every transcript and session here is synthetic.
package presence

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

type goldSession struct {
	In           []byte   `json:"in"`
	OK           bool     `json:"ok"`
	PID          int      `json:"pid"`
	ID           string   `json:"id"`
	Cwd          string   `json:"cwd"`
	Name         string   `json:"name"`
	NameSource   string   `json:"nameSource"`
	Status       string   `json:"status"`
	Kind         string   `json:"kind"`
	Entrypoint   string   `json:"entrypoint"`
	Version      string   `json:"version"`
	PeerFeatures []string `json:"peerFeatures"`
	ProcStart    uint64   `json:"procStart"`
	StartedAt    string   `json:"startedAt"`
	UpdatedAt    string   `json:"updatedAt"`
}

type goldTail struct {
	Src            []byte `json:"src"`
	N              int64  `json:"n"`
	Title          string `json:"title"`
	Custom         string `json:"custom"`
	AI             string `json:"ai"`
	LastText       string `json:"lastText"`
	LastActivity   string `json:"lastActivity"`
	PermissionMode string `json:"permissionMode"`
	Mode           string `json:"mode"`
}

type goldKV struct {
	In  string `json:"in"`
	Out string `json:"out"`
}

type goldOffset struct {
	Unix   int64 `json:"unix"`
	Offset int   `json:"offset"`
}

func goldUTC(t time.Time) string {
	if t.IsZero() {
		return ""
	}
	return t.UTC().Format(time.RFC3339Nano)
}

func TestGoldenPresence(t *testing.T) {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	var sessions []goldSession
	for _, in := range []string{
		`{"pid":13704,"sessionId":"a","cwd":"C:\\Users\\pat","startedAt":1793058851218,"procStart":"134375320001234567","updatedAt":1793063865535,"statusUpdatedAt":1793061536254}`,
		`{"pid":"7","sessionId":"abc","procStart":12,"peerFeatures":[1,"x",null,"y"],"new":{"a":1}}`,
		`{"pid":7.9,"sessionId":"b","procStart":-5}`, `{"pid":7,"sessionId":"b","procStart":12.7}`,
		`{"pid":7,"sessionId":"b","procStart":"Mon Oct 27 14:29:03 2026"}`,
		`{"pid":7,"sessionId":"b","procStart":"wed oct  7 9:05:00 2026"}`,
		`{"pid":7,"sessionId":"b","procStart":"Mon Oct 27 14:29:03 1999"}`,
		`{"pid":7,"sessionId":"b","procStart":"+12"}`, `{"pid":7,"sessionId":"b","procStart":"99999999999999999999"}`,
		`{"pid":7,"sessionId":"b","procStart":true}`,
		`{"pid":"7.5","sessionId":"b","startedAt":"1793058851218","updatedAt":"x","statusUpdatedAt":-1}`,
		`{"pid":7,"sessionId":"b","startedAt":1793058851218.9,"updatedAt":5,"statusUpdatedAt":1793061536254}`,
		`{"PID":7,"sessionId":"b"}`, `{"pid":7,"sessionId":"b","sessionId":"c"}`, `{"pid":7,"sessionId":""}`,
		`{"pid":-1,"sessionId":"b"}`, `{"pid":7,"sessionId":"b","name":"n","nameSource":"user","status":"busy","kind":"bg","entrypoint":"cli","version":"2.1.310"}`,
		`{"pid":7,"sessionId":"b","cwd":"C:\\x\u00e9\ud83d\ude00"}`, "{\"pid\":7,\"sessionId\":\"b\",\"cwd\":\"\xff\"}",
		``, `{`, `[1,2]`, `null`, `{"pid":5}`, `{"pid":5,"sessionId":"a","cwd":"C:\Users"}`,
	} {
		s, ok := readSession(strings.NewReader(in))
		sessions = append(sessions, goldSession{[]byte(in), ok, s.PID, s.ID, s.Cwd, s.Name, s.NameSource, s.Status, s.Kind,
			s.Entrypoint, s.Version, s.PeerFeatures, s.ProcStart, goldUTC(s.StartedAt), goldUTC(s.UpdatedAt)})
	}

	srcs := []string{
		modeRow("default") + assistantRow("2026-10-26T10:00:00Z", "one") + modeRow("bypassPermissions"),
		`{"type":"ai-title","aiTitle":"Fix it"}` + "\n" + `{"type":"custom-title","customTitle":"Bob"}` + "\n" +
			`{"type":"custom-title","customTitle":""}` + "\n" + `{"type":"ai-title","aiTitle":"Later"}` + "\n",
		assistantRow("2026-10-26T10:00:00.5-05:00", "  multi\n\nline\ttext  ") + assistantRow("2026-10-26T09:00:00Z", "older"),
		`{"type":"assistant","isSidechain":true,"timestamp":"2026-10-26T11:00:00Z","message":{"content":[{"type":"text","text":"side"}]}}` + "\n",
		`{"type":"assistant","timestamp":"2026-10-26T10:00:00Z","message":{"content":"a string"}}` + "\n",
		`{"type":"assistant","timestamp":"2026-10-26T10:00:00Z","message":{"content":[{"type":"text","text":"a"},{"type":"text","text":"  "},{"type":"tool_use"}]}}` + "\n",
		`{"type":"assistant","timestamp":"2026-10-26T10:00:00Z","message":{"content":[{"type":"text","text":5}]}}` + "\n",
		`{"type":"user","timestamp":"2026-10-26T10:00:00Z","isSidechain":"no","permissionMode":"plan"}` + "\n" +
			`{"type":"user","timestamp":"2026-10-26T10:00:01Z","permissionMode":"acceptEdits","message":{"content":"x"}}` + "\r\n",
		`{"TYPE":"ai-title","AITITLE":"folded keys"}` + "\n" + `{"type":"ai-title","aiTitle":"a","aititle":"b"}` + "\n",
		"  \u00a0{\"type\":\"ai-title\",\"aiTitle\":\"nbsp\"}\u00a0 \n" + "not json\n{broken\n",
		`{"type":"assistant","timestamp":"bad","message":{"content":[{"type":"text","text":"` + strings.Repeat("é", 300) + `"}]}}` + "\n",
		"{\"type\":\"ai-title\",\"aiTitle\":\"bad \xff utf8\"}\n",
		`{"type":"assistant","timestamp":"2026-10-26T10:00:00Z","message":null}` + "\n" + `{"type":"assistant","timestamp":"2026-10-26T10:00:02Z"}`,
		strings.Repeat("x", 300),
		"",
	}
	var tails []goldTail
	for _, src := range srcs {
		size := int64(len(src))
		for _, n := range []int64{1 << 20, size, size - 1, size / 2, 40, 1, 0} {
			if n < 0 {
				continue
			}
			tl := readTail(strings.NewReader(src), size, n)
			mode := scanMode(strings.NewReader(src), size, n)
			tails = append(tails, goldTail{[]byte(src), n, tl.Title, tl.Custom, tl.AI, tl.LastText, goldUTC(tl.LastActivity), tl.PermissionMode, mode})
		}
	}

	var lstart []goldKV
	for _, in := range []string{"Mon Oct 27 14:29:03 2026", "Wed Oct  7 09:05:00 2026", "  Wed Oct  7 09:05:00 2026\n",
		"wed OCT 7 9:05:00 2026", "Wed Oct 07 09:05:00 2026", "Wed Oct 7 09:5:00 2026", "Wed Oct 7 24:05:00 2026",
		"Wed Feb 29 09:05:00 2027", "Wed Feb 29 09:05:00 2028", "Xyz Oct 7 09:05:00 2026", "Mon Oct 27 14:29:03 1999",
		"Mon Oct 27 14:29:03 2026 extra", "134375320001234567", "", "Mon\u00a0Oct\u300027 14:29:03 2026"} {
		got, ok := parseLstart(in)
		out := "!"
		if ok {
			out = got.UTC().Format(time.RFC3339Nano)
		}
		lstart = append(lstart, goldKV{in, out})
	}

	var slugs, lines, names, sessFiles []goldKV
	for _, in := range []string{`C:\Users\pat`, "/tmp/café", "日本", "a.b_c-d", "", "\u212a"} {
		slugs = append(slugs, goldKV{in, Slug(in)})
	}
	for _, in := range []string{" a\n\n b\t c ", strings.Repeat("é", 300), strings.Repeat("ab ", 120), "x\u00a0y\u2028z",
		strings.Repeat("a", 239) + " b", strings.Repeat("a", 240), strings.Repeat("a", 241), ""} {
		lines = append(lines, goldKV{in, oneLine(in, maxLastText)})
	}
	for _, s := range []session{{ID: "abcdef123456"}, {ID: "日本語のid"}, {ID: "s", Cwd: `C:\work\api\`}, {ID: "s", Cwd: "/a/b/"},
		{ID: "s", Cwd: `\\`}, {ID: "s", Name: "n", Cwd: "/x"}} {
		b, _ := json.Marshal(map[string]string{"id": s.ID, "cwd": s.Cwd, "name": s.Name})
		names = append(names, goldKV{string(b), displayName(s, "")})
	}
	for _, in := range []string{"13704.json", "x.json", ".json", "1.JSON", "1.json.key", "０.json"} {
		sessFiles = append(sessFiles, goldKV{in, fmt.Sprint(isSessionFile(in))})
	}

	var js []goldKV
	add := func(name string, v any) {
		b, err := json.Marshal(v)
		if err != nil {
			t.Fatal(err)
		}
		js = append(js, goldKV{name, string(b)})
	}
	add("cardZero", AgentCard{})
	add("changeZero", Change{})
	at := time.Date(2026, 10, 29, 8, 7, 6, 500000000, time.UTC)
	add("cardFull", AgentCard{Key: "d/s", DeviceID: "d", Agent: "claude", SessionID: "s", PID: 42, Name: "<n>&",
		Title: "t\u2028", NameSource: SourceIrori, AITitle: "ai", Cwd: `C:\x`, Status: "busy", SessionKind: "bg",
		Entrypoint: "cli", Version: "2.1.310", PeerFeatures: []string{"a", "b"}, StartedAt: at, UpdatedAt: at.Add(time.Second),
		LastActivity: at.Add(time.Millisecond), LastText: "last", Transcript: `C:\t.jsonl`, RepoKey: "remote:x/y",
		PermissionMode: "bypassPermissions", DeviceState: "recent"})
	add("changeRemoved", Change{Kind: Removed, Card: AgentCard{Key: "d/s", PeerFeatures: []string{}}})

	// Scan this repo's fixtures (crates/presence/testdata, made-up
	// sessions), not the Go checkout's own testdata: IRORI_GOLDEN_OUT is
	// <repo>/testdata/golden.
	abs, err := filepath.Abs(filepath.Join(dir, "..", "..", "crates", "presence", "testdata"))
	if err != nil {
		t.Fatal(err)
	}
	w := New(Options{ClaudeDir: abs, DeviceID: "dev1"})
	w.impl.live = func(int, uint64) bool { return true }
	w.impl.scan()
	for i, c := range w.Snapshot() {
		c.StartedAt, c.UpdatedAt, c.LastActivity = c.StartedAt.UTC(), c.UpdatedAt.UTC(), c.LastActivity.UTC()
		c.Transcript = strings.ReplaceAll(c.Transcript, abs, "$TESTDATA")
		add(fmt.Sprintf("fixture%d", i), c)
	}

	ref := time.Date(2026, 10, 29, 12, 0, 0, 0, time.UTC)
	var local []goldOffset
	for _, u := range []time.Time{ref, time.Date(2026, 7, 1, 12, 0, 0, 0, time.UTC), time.Date(2026, 1, 15, 12, 0, 0, 0, time.UTC),
		time.Date(2026, 3, 8, 7, 59, 0, 0, time.UTC), time.Date(2026, 3, 8, 8, 1, 0, 0, time.UTC),
		time.Date(2026, 11, 1, 6, 59, 0, 0, time.UTC), time.Date(2026, 11, 1, 7, 1, 0, 0, time.UTC),
		time.Date(2010, 7, 1, 0, 0, 0, 0, time.UTC), time.Date(2040, 1, 1, 0, 0, 0, 0, time.UTC),
		time.UnixMilli(1793058851218)} {
		_, off := u.Local().Zone()
		local = append(local, goldOffset{u.Unix(), off})
	}

	out := map[string]any{"sessions": sessions, "tails": tails, "lstart": lstart, "slug": slugs, "oneLine": lines,
		"displayName": names, "isSessionFile": sessFiles, "json": js, "local": local}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(out); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "presence", "presence.json")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

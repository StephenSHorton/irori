//go:build windows

package api_test

// Injected into I:internal/api with `go test -overlay` by
// tools/goldens/export.sh (architecture.md §6.2, §8 M4): a fresh Go Core
// over a scratch data dir and a fake Claude dir with two live sessions
// (alpha and beta, as core's newRig), on a FakeNet (no network at all),
// with its own api.Server on 127.0.0.1:0 so the person's secret is at
// hand. It runs testdata/golden/api/scenario.json step by step and writes
// every answer to testdata/golden/api/go.json, with each run's random
// values (device id, token, secrets, port, pid, temp dir) replaced by
// their {name}. Windows only, as Claude Code's procStart is (export.sh
// runs there). crates/api/tests/api_golden.rs runs the same scenario on
// the Rust Core and compares.

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/api"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/proc"
)

const (
	goldenSessA = "aaaaaaaa-1111-4111-8111-000000000001"
	goldenSessB = "bbbbbbbb-2222-4222-8222-000000000002"
)

type gStep struct {
	ID  string `json:"id"`
	Req *struct {
		Method  string            `json:"method"`
		Path    string            `json:"path"`
		Headers map[string]string `json:"headers"`
		Body    *string           `json:"body"`
		Auth    *bool             `json:"auth"`
	} `json:"req"`
	Open *struct {
		Stream  string            `json:"stream"`
		Path    string            `json:"path"`
		Headers map[string]string `json:"headers"`
	} `json:"open"`
	Read *struct {
		Stream string `json:"stream"`
		N      int    `json:"n"`
		Until  string `json:"until"`
		Drain  int    `json:"drain"`
	} `json:"read"`
	Close string            `json:"close"`
	Act   map[string]string `json:"act"`
	Save  map[string][]any  `json:"save"`
	Needs string            `json:"needs"`
}

type gOut struct {
	ID      string            `json:"id"`
	Status  int               `json:"status,omitempty"`
	Headers map[string]string `json:"headers,omitempty"`
	Body    *string           `json:"body,omitempty"`
	Lines   []string          `json:"lines,omitempty"`
	Result  *string           `json:"result,omitempty"`
}

// The headers both sides record.
var goldenHeaders = []string{"Content-Type", "Cache-Control", "X-Content-Type-Options", "Allow", "WWW-Authenticate"}

type gStream struct {
	lines  chan string
	cancel context.CancelFunc
}

func TestGoldenAPI(t *testing.T) {
	out := os.Getenv("IRORI_GOLDEN_OUT")
	if out == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	raw, err := os.ReadFile(filepath.Join(out, "api", "scenario.json"))
	if err != nil {
		t.Fatal(err)
	}
	var sc struct {
		Steps []gStep `json:"steps"`
	}
	if err := json.Unmarshal(raw, &sc); err != nil {
		t.Fatal(err)
	}

	data, claude, cwd := t.TempDir(), t.TempDir(), t.TempDir()
	t.Setenv("IRORI_HOME", data)
	sessions := filepath.Join(claude, "sessions")
	if err := os.MkdirAll(sessions, 0o700); err != nil {
		t.Fatal(err)
	}
	// One time for both, so the roster orders them by key (crates/api's
	// rig does the same).
	now := time.Now().UnixMilli()
	writeGoldenSession(t, sessions, "900001.json", goldenSessA, "alpha", cwd, now)
	writeGoldenSession(t, sessions, "900002.json", goldenSessB, "beta", cwd, now)
	c, err := core.Start(context.Background(), core.Options{DataDir: data, ClaudeDir: claude, Version: "0.2.0-test",
		Headless: true, Keystore: identity.FileKeystore{}, Net: mesh.NewFakeNet()})
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	dev := c.Device()
	keyA, keyB := dev.ID+"/"+goldenSessA, dev.ID+"/"+goldenSessB
	deadline := time.Now().Add(10 * time.Second)
	for n := 0; n < 2; {
		n = 0
		for _, card := range c.Agents() {
			if card.Key == keyA || card.Key == keyB {
				n++
			}
		}
		if time.Now().After(deadline) {
			t.Fatalf("presence never listed both sessions")
		}
		time.Sleep(50 * time.Millisecond)
	}

	token, err := discovery.NewToken()
	if err != nil {
		t.Fatal(err)
	}
	srv := api.New(c, token)
	ln, err := api.Listen()
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	served := make(chan struct{})
	go func() { defer close(served); _ = srv.Serve(ctx, ln) }()
	defer func() { cancel(); <-served }()
	port := strconv.Itoa(ln.Addr().(*net.TCPAddr).Port)

	vars := map[string]string{
		"device": dev.ID, "port": port, "token": token, "person": srv.PersonSecret(),
		"keyA": keyA, "keyB": keyB, "sessA": goldenSessA, "sessB": goldenSessB,
		"cwd": filepath.ToSlash(cwd), "cwdNative": cwd, "pid": strconv.Itoa(os.Getpid()),
		"personId": dev.PersonID, "devicePub": dev.DevicePub, "deviceName": dev.Name, "data": data, "claude": claude,
	}
	expand := func(s string) string {
		for k, v := range vars {
			s = strings.ReplaceAll(s, "{"+k+"}", v)
		}
		return s
	}
	// Each run's values back to their names, longest first; the fixed
	// session ids stay as they are, and the port and pid only where they
	// stand as one (not inside a longer number).
	mask := func(s string) string {
		s = strings.ReplaceAll(s, "127.0.0.1:"+port, "127.0.0.1:{port}")
		s = strings.ReplaceAll(s, `"pid":`+vars["pid"]+",", `"pid":{pid},`)
		type kv struct{ k, v string }
		var list []kv
		for k, v := range vars {
			if v != "" && k != "sessA" && k != "sessB" && k != "keyA" && k != "keyB" && k != "port" && k != "pid" {
				list = append(list, kv{k, v})
				if e, err := json.Marshal(v); err == nil && string(e[1:len(e)-1]) != v {
					list = append(list, kv{k, string(e[1 : len(e)-1])})
				}
			}
		}
		sort.Slice(list, func(i, j int) bool { return len(list[i].v) > len(list[j].v) })
		for _, p := range list {
			s = strings.ReplaceAll(s, p.v, "{"+p.k+"}")
		}
		return s
	}

	client := &http.Client{Transport: &http.Transport{Proxy: nil}}
	base := "http://127.0.0.1:" + port
	streams := map[string]*gStream{}
	defer func() {
		for _, s := range streams {
			s.cancel()
		}
	}()
	build := func(method, path string, headers map[string]string, body *string, auth bool) *http.Request {
		var rd io.Reader
		if body != nil {
			rd = strings.NewReader(expand(*body))
		}
		req, err := http.NewRequest(method, base+expand(path), rd)
		if err != nil {
			t.Fatal(err)
		}
		if auth {
			req.Header.Set("Authorization", "Bearer "+token)
		}
		for k, v := range headers {
			if k == "Host" {
				req.Host = expand(v)
				continue
			}
			req.Header[http.CanonicalHeaderKey(k)] = []string{expand(v)}
		}
		return req
	}
	save := func(st gStep, docs []string) {
		for name, path := range st.Save {
			i := 0
			keys := path
			if len(path) > 0 {
				if n, ok := path[0].(float64); ok {
					i, keys = int(n), path[1:]
					if i < 0 {
						i += len(docs)
					}
				}
			}
			var v any
			if err := json.Unmarshal([]byte(docs[i]), &v); err != nil {
				t.Fatalf("%s: save %s: %v", st.ID, name, err)
			}
			for _, k := range keys {
				v = v.(map[string]any)[k.(string)]
			}
			vars[name] = v.(string)
		}
	}

	var outs []gOut
	for _, st := range sc.Steps {
		o := gOut{ID: st.ID}
		switch {
		case st.Req != nil:
			auth := st.Req.Auth == nil || *st.Req.Auth
			resp, err := client.Do(build(st.Req.Method, st.Req.Path, st.Req.Headers, st.Req.Body, auth))
			if err != nil {
				t.Fatalf("%s: %v", st.ID, err)
			}
			b, _ := io.ReadAll(resp.Body)
			resp.Body.Close()
			o.Status = resp.StatusCode
			o.Headers = map[string]string{}
			for _, h := range goldenHeaders {
				if v := resp.Header.Values(h); len(v) > 0 {
					o.Headers[h] = strings.Join(v, ", ")
				}
			}
			if len(st.Save) > 0 && resp.StatusCode == 200 {
				save(st, []string{string(b)})
			}
			body := mask(string(b))
			o.Body = &body
		case st.Open != nil:
			sctx, scancel := context.WithCancel(context.Background())
			req := build("GET", st.Open.Path, st.Open.Headers, nil, true).WithContext(sctx)
			resp, err := client.Do(req)
			if err != nil {
				scancel()
				t.Fatalf("%s: %v", st.ID, err)
			}
			o.Status = resp.StatusCode
			o.Headers = map[string]string{}
			for _, h := range goldenHeaders {
				if v := resp.Header.Values(h); len(v) > 0 {
					o.Headers[h] = strings.Join(v, ", ")
				}
			}
			if resp.StatusCode != 200 {
				b, _ := io.ReadAll(resp.Body)
				resp.Body.Close()
				scancel()
				body := mask(string(b))
				o.Body = &body
				break
			}
			s := &gStream{lines: make(chan string, 256), cancel: scancel}
			streams[st.Open.Stream] = s
			go func() {
				defer close(s.lines)
				defer resp.Body.Close()
				sc := bufio.NewScanner(resp.Body)
				sc.Buffer(make([]byte, 0, 64<<10), 8<<20)
				for sc.Scan() {
					if line := sc.Text(); len(bytes.TrimSpace([]byte(line))) > 0 {
						s.lines <- line
					}
				}
			}()
		case st.Read != nil:
			s := streams[st.Read.Stream]
			if s == nil {
				t.Fatalf("%s: no stream %q", st.ID, st.Read.Stream)
			}
			var got []string
			wait := 10 * time.Second
			if st.Read.Drain > 0 {
				wait = time.Duration(st.Read.Drain) * time.Millisecond
			}
		read:
			for st.Read.N == 0 || len(got) < st.Read.N {
				select {
				case line, ok := <-s.lines:
					if !ok {
						if st.Read.Drain > 0 {
							break read
						}
						t.Fatalf("%s: stream %s ended after %d lines", st.ID, st.Read.Stream, len(got))
					}
					got = append(got, line)
					if st.Read.Until != "" && strings.Contains(line, `{"type":"`+st.Read.Until+`"`) {
						break read
					}
				case <-time.After(wait):
					if st.Read.Drain > 0 {
						break read
					}
					t.Fatalf("%s: stream %s: no line in %v after %d", st.ID, st.Read.Stream, wait, len(got))
				}
			}
			if len(st.Save) > 0 {
				save(st, got)
			}
			for _, l := range got {
				o.Lines = append(o.Lines, mask(l))
			}
			if o.Lines == nil {
				o.Lines = []string{}
			}
		case st.Close != "":
			if s := streams[st.Close]; s != nil {
				s.cancel()
				delete(streams, st.Close)
			}
		case st.Act != nil:
			a := map[string]string{}
			for k, v := range st.Act {
				a[k] = expand(v)
			}
			var res any
			switch a["do"] {
			case "send":
				m, d, err := c.Inbox().Send(context.Background(), inbox.Sender{Key: a["from"], Name: a["fromName"], Kind: "claude"},
					inbox.Address{Kind: inbox.ToAgent, Value: a["to"]}, a["text"], "")
				if err != nil {
					t.Fatalf("%s: %v", st.ID, err)
				}
				res = map[string]string{"message": m.ID, "delivery": d.ID, "state": string(d.State)}
			case "task":
				tk, err := c.Coord().CreateTask(context.Background(), a["by"], coord.NewTask{Title: a["title"]})
				if err != nil {
					t.Fatalf("%s: %v", st.ID, err)
				}
				res = map[string]string{"id": tk.ID}
			case "comment":
				cm, _, err := c.Coord().AddComment(context.Background(), a["by"], a["task"], a["text"], false)
				if err != nil {
					t.Fatalf("%s: %v", st.ID, err)
				}
				res = map[string]string{"id": cm.ID}
			default:
				t.Fatalf("%s: unknown act %q", st.ID, a["do"])
			}
			b, _ := json.Marshal(res)
			if len(st.Save) > 0 {
				save(st, []string{string(b)})
			}
			r := mask(string(b))
			o.Result = &r
		}
		outs = append(outs, o)
	}

	b, err := json.MarshalIndent(map[string]any{"steps": outs}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(out, "api", "go.json"), append(b, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
	fmt.Fprintf(os.Stderr, "api golden: %d steps\n", len(outs))
}

func writeGoldenSession(t *testing.T, dir, file, id, name, cwd string, at int64) {
	t.Helper()
	m := map[string]any{
		"pid": os.Getpid(), "sessionId": id, "cwd": cwd, "name": name,
		"status": "idle", "kind": "bg", "entrypoint": "cli", "version": "2.1.311",
		"startedAt": at, "updatedAt": at,
	}
	if st, ok := proc.StartTime(os.Getpid()); ok && proc.FiletimeTicks(st) != 0 {
		m["procStart"] = strconv.FormatUint(proc.FiletimeTicks(st), 10)
	}
	raw, _ := json.Marshal(m)
	if err := os.WriteFile(filepath.Join(dir, file), raw, 0o600); err != nil {
		t.Fatal(err)
	}
}

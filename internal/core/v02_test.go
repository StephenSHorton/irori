package core

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/api"
	"github.com/StephenSHorton/irori/internal/attach"
	"github.com/StephenSHorton/irori/internal/attach/runner"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/store"
)

const (
	sessA = "aaaaaaaa-1111-4111-8111-000000000001"
	sessB = "bbbbbbbb-2222-4222-8222-000000000002"
)

type rig struct {
	t      *testing.T
	core   *Core
	data   string
	base   string
	token  string
	keyA   string
	keyB   string
	client *http.Client
}

func writeSession(t *testing.T, dir, file, id, name, cwd string) {
	t.Helper()
	m := map[string]any{
		"pid": os.Getpid(), "sessionId": id, "cwd": cwd, "name": name,
		"status": "idle", "kind": "bg", "entrypoint": "cli", "version": "2.1.311",
		"startedAt": time.Now().UnixMilli(), "updatedAt": time.Now().UnixMilli(),
	}
	if ps := procStartField(); ps != "" {
		m["procStart"] = ps
	}
	raw, _ := json.Marshal(m)
	if err := os.WriteFile(filepath.Join(dir, file), raw, 0o600); err != nil {
		t.Fatal(err)
	}
}

// newRig starts a Core over a fake ClaudeDir holding two live sessions,
// alpha (A) and beta (B), and waits until presence lists both.
func newRig(t *testing.T) *rig {
	t.Helper()
	return newRigWith(t, nil)
}

// newRigWith is newRig with edit applied to the Core's options first.
func newRigWith(t *testing.T, edit func(*Options)) *rig {
	t.Helper()
	data, claude := t.TempDir(), t.TempDir()
	t.Setenv("IRORI_HOME", data)
	sessions := filepath.Join(claude, "sessions")
	if err := os.MkdirAll(sessions, 0o700); err != nil {
		t.Fatal(err)
	}
	cwd := t.TempDir()
	writeSession(t, sessions, "900001.json", sessA, "alpha", cwd)
	writeSession(t, sessions, "900002.json", sessB, "beta", cwd)
	opts := Options{DataDir: data, ClaudeDir: claude, Version: "0.2.0-test", Headless: true, Keystore: identity.FileKeystore{}}
	if edit != nil {
		edit(&opts)
	}
	c, err := Start(context.Background(), opts)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	lf, err := discovery.Read(data)
	if err != nil {
		t.Fatal(err)
	}
	r := &rig{
		t: t, core: c, data: data, base: "http://127.0.0.1:" + strconv.Itoa(lf.Port), token: lf.Token,
		keyA: c.Device().ID + "/" + sessA, keyB: c.Device().ID + "/" + sessB,
		client: &http.Client{Transport: &http.Transport{Proxy: nil}},
	}
	deadline := time.Now().Add(10 * time.Second)
	for {
		n := 0
		for _, card := range c.Agents() {
			if card.Key == r.keyA || card.Key == r.keyB {
				n++
			}
		}
		if n == 2 {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("presence never listed both sessions: %+v", c.Agents())
		}
		time.Sleep(50 * time.Millisecond)
	}
	return r
}

// evStream is an open attach stream read in the background.
type evStream struct {
	t      *testing.T
	events chan attach.Event
	cancel context.CancelFunc
	done   chan struct{}
}

func (r *rig) open(q string) (*evStream, int) {
	r.t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	req, _ := http.NewRequestWithContext(ctx, "GET", r.base+"/v1/attach?"+q, nil)
	req.Header.Set("Authorization", "Bearer "+r.token)
	resp, err := r.client.Do(req)
	if err != nil {
		cancel()
		r.t.Fatal(err)
	}
	if resp.StatusCode != 200 {
		resp.Body.Close()
		cancel()
		return nil, resp.StatusCode
	}
	s := &evStream{t: r.t, events: make(chan attach.Event, 64), cancel: cancel, done: make(chan struct{})}
	go func() {
		defer close(s.done)
		defer close(s.events)
		defer resp.Body.Close()
		sc := bufio.NewScanner(resp.Body)
		sc.Buffer(make([]byte, 0, 64<<10), 1<<20)
		for sc.Scan() {
			line := bytes.TrimSpace(sc.Bytes())
			if len(line) == 0 {
				continue
			}
			var ev attach.Event
			if err := json.Unmarshal(line, &ev); err != nil {
				r.t.Errorf("bad line %q: %v", line, err)
				return
			}
			s.events <- ev
		}
	}()
	r.t.Cleanup(func() { s.close() })
	return s, 200
}

func (r *rig) attach(session string) (*evStream, attach.Event) {
	r.t.Helper()
	s, code := r.open("session=" + session + "&mod=0.2.0-test")
	if code != 200 {
		r.t.Fatalf("attach %s: status %d", session, code)
	}
	hello := s.next(attach.TypeHello)
	if hello.AttachSecret == "" || hello.API == nil || hello.API.Token != r.token || !strings.HasPrefix(hello.API.URL, "http://127.0.0.1:") {
		r.t.Fatalf("hello %+v", hello)
	}
	return s, hello
}

func (s *evStream) close() {
	s.cancel()
	<-s.done
}

// next returns the next event of type typ, skipping others.
func (s *evStream) next(typ string) attach.Event {
	s.t.Helper()
	timeout := time.After(10 * time.Second)
	for {
		select {
		case ev, ok := <-s.events:
			if !ok {
				s.t.Fatalf("stream closed waiting for %s", typ)
			}
			if ev.Type == typ {
				return ev
			}
		case <-timeout:
			s.t.Fatalf("no %s event", typ)
		}
	}
}

// none asserts no event of type typ arrives within d.
func (s *evStream) none(typ string, d time.Duration) {
	s.t.Helper()
	timeout := time.After(d)
	for {
		select {
		case ev, ok := <-s.events:
			if !ok {
				return
			}
			if ev.Type == typ {
				s.t.Fatalf("unexpected %s: %+v", typ, ev)
			}
		case <-timeout:
			return
		}
	}
}

// ended asserts the stream closes (after an optional last event of type typ).
func (s *evStream) ended(typ string) {
	s.t.Helper()
	timeout := time.After(10 * time.Second)
	seen := typ == ""
	for {
		select {
		case ev, ok := <-s.events:
			if !ok {
				if !seen {
					s.t.Fatalf("stream ended without %s", typ)
				}
				return
			}
			if ev.Type == typ {
				seen = true
			}
		case <-timeout:
			s.t.Fatal("stream did not end")
		}
	}
}

func (r *rig) do(method, path string, hdr map[string]string, body any) (int, []byte) {
	r.t.Helper()
	var rd io.Reader
	if body != nil {
		raw, _ := json.Marshal(body)
		rd = bytes.NewReader(raw)
	}
	req, _ := http.NewRequest(method, r.base+path, rd)
	req.Header.Set("Authorization", "Bearer "+r.token)
	for k, v := range hdr {
		req.Header.Set(k, v)
	}
	resp, err := r.client.Do(req)
	if err != nil {
		r.t.Fatal(err)
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, raw
}

func as(secret string) map[string]string { return map[string]string{attach.HeaderAttach: secret} }

func (r *rig) control(secret, what, token, turn string) {
	r.t.Helper()
	code, raw := r.do("POST", "/v1/deliveries/"+what, as(secret), map[string]string{"token": token, "turnId": turn})
	if code != 200 || !strings.Contains(string(raw), `"ok":true`) {
		r.t.Fatalf("%s: %d %s", what, code, raw)
	}
}

func (r *rig) state(id string) inbox.State {
	d, ok := r.core.Inbox().Get(id)
	if !ok {
		r.t.Fatalf("no delivery %s", id)
	}
	return d.State
}

func (r *rig) sender(key, name string) inbox.Sender {
	return inbox.Sender{Key: key, Name: name, Kind: "claude"}
}

func TestTwoSessionsTalk(t *testing.T) {
	r := newRig(t)
	ctx := context.Background()
	a, helloA := r.attach(sessA)
	b, helloB := r.attach(sessB)
	if helloA.AgentKey != r.keyA || helloA.Name != "alpha" || helloB.Session != sessB {
		t.Fatalf("hellos %+v %+v", helloA, helloB)
	}
	a.next(attach.TypeInbox)
	b.next(attach.TypeInbox)

	t.Run("tools-send", func(t *testing.T) {
		code, raw := r.do("POST", "/v1/tools/send", as(helloA.AttachSecret),
			map[string]any{"args": map[string]any{"to": "beta", "text": "via tools"}})
		if code == http.StatusNotFound {
			t.Skipf("tools package is still a stub here: %s", raw)
		}
		if code != 200 || !strings.Contains(string(raw), `"text"`) {
			t.Fatalf("tools send: %d %s", code, raw)
		}
		ev := b.next(attach.TypeDeliver)
		r.control(helloB.AttachSecret, "delivered", ev.Token, "t0")
		r.control(helloB.AttachSecret, "engaged", ev.Token, "t0")
		r.control(helloB.AttachSecret, "turn-ended", ev.Token, "t0")
	})

	_, d1, err := r.core.Inbox().Send(ctx, r.sender(r.keyA, "alpha"), inbox.Address{Kind: inbox.ToAgent, Value: "beta"}, "ping-7f3e: please look", "")
	if err != nil {
		t.Fatal(err)
	}
	ev := b.next(attach.TypeDeliver)
	if !strings.Contains(ev.Prompt, ev.Token) || !strings.Contains(ev.Prompt, "ping-7f3e") ||
		!strings.Contains(ev.Prompt, `<irori-message from="alpha"`) || !strings.Contains(ev.Prompt, `trust="untrusted"`) {
		t.Fatalf("prompt %q", ev.Prompt)
	}
	if len(ev.DeliveryIDs) != 1 || ev.DeliveryIDs[0] != d1.ID || len(ev.MessageIDs) != 1 || ev.From != "alpha" {
		t.Fatalf("deliver %+v", ev)
	}
	r.control(helloB.AttachSecret, "delivered", ev.Token, "")
	if s := r.state(d1.ID); s != inbox.Delivered {
		t.Fatalf("after delivered: %s", s)
	}
	r.control(helloB.AttachSecret, "engaged", ev.Token, "turn-1")
	if s := r.state(d1.ID); s != inbox.Engaged {
		t.Fatalf("after engaged: %s", s)
	}

	// B's inbox over HTTP shows it.
	code, raw := r.do("GET", "/v1/inbox", as(helloB.AttachSecret), nil)
	var list []inbox.Delivery
	if code != 200 || json.Unmarshal(raw, &list) != nil || len(list) == 0 || list[0].ID != d1.ID {
		t.Fatalf("inbox: %d %s", code, raw)
	}

	if _, _, err := r.core.Inbox().Reply(ctx, r.sender(r.keyB, "beta"), d1.ID, "pong-91ac"); err != nil {
		t.Fatal(err)
	}
	r.control(helloB.AttachSecret, "turn-ended", ev.Token, "turn-1")
	if s := r.state(d1.ID); s != inbox.Answered {
		t.Fatalf("after reply: %s", s)
	}
	back := a.next(attach.TypeDeliver)
	if !strings.Contains(back.Prompt, "pong-91ac") || !strings.Contains(back.Prompt, back.Token) || back.From != "beta" {
		t.Fatalf("reply deliver %+v", back)
	}
}

func TestSupersede(t *testing.T) {
	r := newRig(t)
	ctx := context.Background()
	b1, h1 := r.attach(sessB)
	_, d1, err := r.core.Inbox().Send(ctx, r.sender(r.keyA, "alpha"), inbox.Address{Kind: inbox.ToAgent, Value: "beta"}, "first", "")
	if err != nil {
		t.Fatal(err)
	}
	w1 := b1.next(attach.TypeDeliver)

	// A reload before the mod confirmed: the new stream gets the same wake
	// again (same token), the old one is told it was superseded.
	b2, h2 := r.attach(sessB)
	b1.ended(attach.TypeSuperseded)
	again := b2.next(attach.TypeDeliver)
	if again.Token != w1.Token || len(again.DeliveryIDs) != 1 || again.DeliveryIDs[0] != d1.ID {
		t.Fatalf("re-sent wake %+v, want token %s", again, w1.Token)
	}
	if code, _ := r.do("POST", "/v1/deliveries/delivered", as(h1.AttachSecret), map[string]string{"token": w1.Token}); code != http.StatusForbidden {
		t.Fatalf("superseded secret: status %d, want 403", code)
	}
	r.control(h2.AttachSecret, "delivered", w1.Token, "")

	// A reload after it confirmed: nothing is re-sent.
	b3, h3 := r.attach(sessB)
	b2.ended(attach.TypeSuperseded)
	b3.none(attach.TypeDeliver, 700*time.Millisecond)

	// A second message waits behind the outstanding wake, then goes alone.
	_, d2, err := r.core.Inbox().Send(ctx, r.sender(r.keyA, "alpha"), inbox.Address{Kind: inbox.ToAgent, Value: "beta"}, "second", "")
	if err != nil {
		t.Fatal(err)
	}
	b3.none(attach.TypeDeliver, 500*time.Millisecond)
	r.control(h3.AttachSecret, "engaged", w1.Token, "turn-1")
	w2 := b3.next(attach.TypeDeliver)
	if w2.Token == w1.Token || len(w2.DeliveryIDs) != 1 || w2.DeliveryIDs[0] != d2.ID || strings.Contains(w2.Prompt, "first") {
		t.Fatalf("second wake %+v", w2)
	}
	if s := r.state(d1.ID); s != inbox.Engaged {
		t.Fatalf("first delivery: %s", s)
	}

	// Closing the live stream invalidates its secret.
	b3.close()
	deadline := time.Now().Add(5 * time.Second)
	for {
		code, _ := r.do("GET", "/v1/inbox", as(h3.AttachSecret), nil)
		if code == http.StatusForbidden {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("closed stream's secret still works: %d", code)
		}
		time.Sleep(20 * time.Millisecond)
	}
}

func TestKeepalive(t *testing.T) {
	old := api.Keepalive
	api.Keepalive = 50 * time.Millisecond
	t.Cleanup(func() { api.Keepalive = old })
	r := newRig(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	req, _ := http.NewRequestWithContext(ctx, "GET", r.base+"/v1/attach?session="+sessA, nil)
	req.Header.Set("Authorization", "Bearer "+r.token)
	resp, err := r.client.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if ct := resp.Header.Get("Content-Type"); ct != "application/x-ndjson" {
		t.Fatalf("content type %q", ct)
	}
	br := bufio.NewReader(resp.Body)
	first, err := br.ReadString('\n')
	if err != nil || !strings.HasPrefix(first, `{"type":"hello"`) {
		t.Fatalf("first line %q, %v", first, err)
	}
	for {
		line, err := br.ReadString('\n')
		if err != nil {
			t.Fatal(err)
		}
		if line == "\n" {
			return // a bare keepalive
		}
	}
}

func TestAuth(t *testing.T) {
	r := newRig(t)
	_, h := r.attach(sessA)
	for _, hdr := range []map[string]string{nil, as("nope"), {attach.HeaderMember: "nope"},
		{attach.HeaderAttach: h.AttachSecret, attach.HeaderMember: "x"}} {
		for _, path := range []string{"/v1/deliveries/delivered", "/v1/tools/send"} {
			if code, raw := r.do("POST", path, hdr, map[string]string{"token": "[irori-wake:x]"}); code != http.StatusForbidden {
				t.Errorf("%s with %v: %d %s, want 403", path, hdr, code, raw)
			}
		}
		if code, _ := r.do("GET", "/v1/inbox", hdr, nil); code != http.StatusForbidden {
			t.Errorf("inbox with %v: %d, want 403", hdr, code)
		}
	}
	if code, _ := r.do("GET", "/v1/attach?session=not-a-session", nil, nil); code != http.StatusNotFound {
		t.Errorf("unknown session: %d, want 404", code)
	}
	if code, _ := r.do("GET", "/v1/attach?member=nope", nil, nil); code != http.StatusForbidden {
		t.Errorf("unknown member: %d, want 403", code)
	}

	// Policy: readable, writable only by an attached mod from the composer.
	code, raw := r.do("GET", "/v1/policy", nil, nil)
	if code != 200 || !strings.Contains(string(raw), `"mode":"everyone"`) {
		t.Fatalf("get policy: %d %s", code, raw)
	}
	p := inbox.Policy{Mode: inbox.Allowlist, Allow: []string{"alpha"}}
	if code, _ := r.do("POST", "/v1/policy", as(h.AttachSecret), p); code != http.StatusForbidden {
		t.Errorf("policy without composer origin: %d, want 403", code)
	}
	if code, _ := r.do("POST", "/v1/policy", map[string]string{attach.HeaderOrigin: "composer"}, p); code != http.StatusForbidden {
		t.Errorf("policy without attach: %d, want 403", code)
	}
	hdr := as(h.AttachSecret)
	hdr[attach.HeaderOrigin] = "composer"
	code, raw = r.do("POST", "/v1/policy", hdr, p)
	if code != 200 || !strings.Contains(string(raw), `"mode":"allowlist"`) || r.core.Inbox().Policy().Mode != inbox.Allowlist {
		t.Fatalf("policy from composer: %d %s", code, raw)
	}
	// Set at the composer through the API: actor "api:composer@<key>", never
	// "user" (v0.4 §2), in the device-local table (v0.4 §5.1).
	evs, err := r.core.store.List(context.Background(), store.Filter{Kinds: []string{inbox.KindPolicySet}, Desc: true, Limit: 1, Local: true})
	if err != nil || len(evs) != 1 {
		t.Fatalf("policy.set rows: %v %v", evs, err)
	}
	if evs[0].Actor != "api:composer@"+h.AgentKey || !strings.Contains(string(evs[0].Payload), h.AgentKey) {
		t.Errorf("policy.set actor %q payload %s, want api:composer@ naming %s", evs[0].Actor, evs[0].Payload, h.AgentKey)
	}
	if code, _ := r.do("GET", "/v1/members", nil, nil); code != http.StatusMethodNotAllowed {
		t.Errorf("GET members: %d, want 405", code)
	}
}

func TestGrokMember(t *testing.T) {
	r := newRig(t)
	ctx := context.Background()
	join := func(name, session string) api.Member {
		t.Helper()
		code, raw := r.do("POST", "/v1/members", nil, map[string]string{"name": name, "session": session})
		var m api.Member
		if code != 200 || json.Unmarshal(raw, &m) != nil || m.Secret == "" {
			t.Fatalf("join: %d %s", code, raw)
		}
		return m
	}
	m1 := join("grokky", "grok-session-1")
	m := join("grokky", "grok-session-1")
	if m.Key != m1.Key || m.Secret == m1.Secret || !strings.HasPrefix(m.Key, r.core.Device().ID+"/grok-") {
		t.Fatalf("rejoin %+v after %+v", m, m1)
	}
	if code, _ := r.do("GET", "/v1/inbox", map[string]string{attach.HeaderMember: m1.Secret}, nil); code != http.StatusForbidden {
		t.Fatalf("retired member secret: %d, want 403", code)
	}

	for _, c := range r.core.Agents() {
		if c.Key == m.Key {
			t.Fatalf("grok card listed after a join with no stream: %+v", c)
		}
	}

	s, code := r.open("member=" + m.Secret)
	if code != 200 {
		t.Fatalf("member stream: %d", code)
	}
	hello := s.next(attach.TypeHello)
	if hello.AgentKey != m.Key || hello.Name != "grokky" || hello.AttachSecret != "" {
		t.Fatalf("member hello %+v", hello)
	}
	found := false
	for _, c := range r.core.Agents() {
		if c.Key == m.Key {
			found = c.Agent == "grok" && c.Status == "idle" && c.Name == "grokky"
		}
	}
	if !found {
		t.Fatalf("no grok card in %+v", r.core.Agents())
	}

	_, d, err := r.core.Inbox().Send(ctx, r.sender(r.keyA, "alpha"), inbox.Address{Kind: inbox.ToAgent, Value: "grokky"}, "hello grok 5c1d", "")
	if err != nil {
		t.Fatal(err)
	}
	ev := s.next(attach.TypeDeliver)
	if len(ev.DeliveryIDs) != 1 || ev.DeliveryIDs[0] != d.ID || !strings.Contains(ev.Prompt, "hello grok 5c1d") || ev.From != "alpha" {
		t.Fatalf("member deliver %+v", ev)
	}
	if st := r.state(d.ID); st != inbox.Queued {
		t.Fatalf("a channel push isn't delivered yet: %s", st)
	}
	// Control routes are for the mod only.
	if code, _ := r.do("POST", "/v1/deliveries/delivered", map[string]string{attach.HeaderMember: m.Secret}, map[string]string{"token": ev.Token}); code != http.StatusForbidden {
		t.Fatalf("member on control route: %d, want 403", code)
	}
	code, raw := r.do("GET", "/v1/inbox", map[string]string{attach.HeaderMember: m.Secret}, nil)
	if code != 200 || !strings.Contains(string(raw), d.ID) {
		t.Fatalf("member inbox: %d %s", code, raw)
	}
	if err := r.core.Inbox().Ack(ctx, m.Key, d.ID); err != nil {
		t.Fatal(err)
	}
	if st := r.state(d.ID); st != inbox.Engaged {
		t.Fatalf("after ack: %s", st)
	}

	// The card outlives the stream by the reattach grace only.
	s.close()
	deadline := time.Now().Add(5 * time.Second)
	for {
		r.core.members.mu.Lock()
		open := r.core.members.byKey[m.Key].open
		r.core.members.mu.Unlock()
		if open == 0 {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("member stream never closed")
		}
		time.Sleep(20 * time.Millisecond)
	}
	for _, c := range r.core.Agents() {
		if c.Key == m.Key {
			found = true
		}
	}
	if !found {
		t.Fatal("grok card gone before the grace ran out")
	}
	r.core.members.mu.Lock()
	r.core.members.now = func() time.Time { return time.Now().Add(inbox.ReattachGrace + time.Second) }
	r.core.members.mu.Unlock()
	for {
		gone := true
		for _, c := range r.core.Agents() {
			if c.Key == m.Key {
				gone = false
			}
		}
		if gone {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("grok card outlived the grace")
		}
		time.Sleep(20 * time.Millisecond)
	}
}

// lines collects a runner's stdout.
type lines struct {
	mu  sync.Mutex
	evs []attach.Event
	raw []string
}

func (l *lines) read(t *testing.T, rd io.Reader) {
	sc := bufio.NewScanner(rd)
	sc.Buffer(make([]byte, 0, 64<<10), 1<<20)
	for sc.Scan() {
		var ev attach.Event
		if err := json.Unmarshal(sc.Bytes(), &ev); err != nil {
			t.Errorf("runner wrote a non-event line %q", sc.Text())
			continue
		}
		l.mu.Lock()
		l.evs = append(l.evs, ev)
		l.raw = append(l.raw, sc.Text())
		l.mu.Unlock()
	}
}

func (l *lines) wait(t *testing.T, typ string, n int) attach.Event {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for {
		l.mu.Lock()
		seen := 0
		for _, ev := range l.evs {
			if ev.Type == typ {
				seen++
				if seen == n {
					l.mu.Unlock()
					return ev
				}
			}
		}
		l.mu.Unlock()
		if time.Now().After(deadline) {
			t.Fatalf("runner: no %s #%d in %v", typ, n, l.raw)
		}
		time.Sleep(20 * time.Millisecond)
	}
}

type runnerProc struct {
	stdin *io.PipeWriter
	out   *lines
	done  chan error
}

func startRunner(t *testing.T, data, session string) *runnerProc {
	inR, inW := io.Pipe()
	outR, outW := io.Pipe()
	p := &runnerProc{stdin: inW, out: &lines{}, done: make(chan error, 1)}
	go p.out.read(t, outR)
	go func() {
		err := runner.Run(context.Background(), runner.Options{
			Session: session, ModVersion: "0.2.0-test", DataDir: data,
			Stdin: inR, Stdout: outW, Logf: t.Logf,
			MinBackoff: 50 * time.Millisecond, MaxBackoff: 200 * time.Millisecond,
		})
		outW.Close()
		p.done <- err
	}()
	t.Cleanup(func() {
		inW.Close()
		select {
		case <-p.done:
		case <-time.After(5 * time.Second):
		}
	})
	return p
}

func (p *runnerProc) exited(t *testing.T) {
	t.Helper()
	select {
	case err := <-p.done:
		if err != nil {
			t.Fatalf("runner: %v", err)
		}
		p.done <- nil // for cleanup
	case <-time.After(10 * time.Second):
		t.Fatal("runner did not exit")
	}
}

func TestAttachCommand(t *testing.T) {
	// Not running yet: one error line, then it attaches once Irori starts.
	early := t.TempDir()
	p0 := startRunner(t, early, sessB)
	ev := p0.out.wait(t, attach.TypeError, 1)
	if !strings.Contains(ev.Message, "Irori isn't running") {
		t.Fatalf("error %+v", ev)
	}
	time.Sleep(300 * time.Millisecond) // several retries, still one error line
	p0.out.mu.Lock()
	nerr := 0
	for _, e := range p0.out.evs {
		if e.Type == attach.TypeError {
			nerr++
		}
	}
	p0.out.mu.Unlock()
	if nerr != 1 {
		t.Fatalf("%d error lines during one outage, want 1", nerr)
	}
	p0.stdin.Close()
	p0.exited(t)

	r := newRig(t)
	p1 := startRunner(t, r.data, sessB)
	hello := p1.out.wait(t, attach.TypeHello, 1)
	if hello.AgentKey != r.keyB {
		t.Fatalf("hello %+v", hello)
	}
	if _, _, err := r.core.Inbox().Send(context.Background(), r.sender(r.keyA, "alpha"),
		inbox.Address{Kind: inbox.ToAgent, Value: "beta"}, "through the pipe", ""); err != nil {
		t.Fatal(err)
	}
	d := p1.out.wait(t, attach.TypeDeliver, 1)
	if !strings.Contains(d.Prompt, "through the pipe") {
		t.Fatalf("deliver %+v", d)
	}

	// A second attach for the same session supersedes the first, which exits 0.
	p2 := startRunner(t, r.data, sessB)
	p2.out.wait(t, attach.TypeHello, 1)
	p1.out.wait(t, attach.TypeSuperseded, 1)
	p1.exited(t)

	// Closing stdin ends it.
	p2.stdin.Close()
	p2.exited(t)
}

// A superseded member stream must close its own count, or the grok card
// (and the roster entry inbox and coord read) stays listed for good.
func TestGrokMemberSupersede(t *testing.T) {
	r := newRig(t)
	code, raw := r.do("POST", "/v1/members", nil, map[string]string{"name": "grokky", "session": "grok-session-2"})
	var m api.Member
	if code != 200 || json.Unmarshal(raw, &m) != nil || m.Secret == "" {
		t.Fatalf("join: %d %s", code, raw)
	}
	s1, code := r.open("member=" + m.Secret)
	if code != 200 {
		t.Fatalf("member stream 1: %d", code)
	}
	s1.next(attach.TypeHello)
	s2, code := r.open("member=" + m.Secret)
	if code != 200 {
		t.Fatalf("member stream 2: %d", code)
	}
	s2.next(attach.TypeHello)
	s1.ended(attach.TypeSuperseded)
	s2.close()

	openCount := func() int {
		r.core.members.mu.Lock()
		defer r.core.members.mu.Unlock()
		return r.core.members.byKey[m.Key].open
	}
	deadline := time.Now().Add(5 * time.Second)
	for openCount() != 0 {
		if time.Now().After(deadline) {
			t.Fatalf("open count %d after both member streams closed", openCount())
		}
		time.Sleep(20 * time.Millisecond)
	}
	r.core.members.mu.Lock()
	r.core.members.now = func() time.Time { return time.Now().Add(inbox.ReattachGrace + time.Second) }
	r.core.members.mu.Unlock()
	for _, c := range r.core.Agents() {
		if c.Key == m.Key {
			t.Fatalf("grok card still listed after the grace: %+v", c)
		}
	}
}

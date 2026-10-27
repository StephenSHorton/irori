package mcpserver

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/StephenSHorton/irori/internal/attach"
	"github.com/StephenSHorton/irori/internal/tools"
)

const fakeToken = "test-token"

type fakeCall struct {
	Name, Member, Body string
}

type fakeJoin struct {
	Name, Session string
}

// fakeCore serves the routes_v02.go contract over httptest.
type fakeCore struct {
	t  *testing.T
	ts *httptest.Server

	mu       sync.Mutex
	specs    []tools.Spec // nil answers 404, like a v0.1 Core
	calls    []fakeCall
	joins    []fakeJoin
	secrets  map[string]string // live secret -> member key
	keys     map[string]string // session -> member key
	n        int
	attaches []string
	streamOn chan string // secret, each time a stream opens

	events chan attach.Event // fed to whichever stream is open
	cut    chan struct{}     // closes the open stream
}

func newFakeCore(t *testing.T, specs []tools.Spec) *fakeCore {
	f := &fakeCore{t: t, specs: specs, secrets: map[string]string{}, keys: map[string]string{},
		streamOn: make(chan string, 16), events: make(chan attach.Event, 16), cut: make(chan struct{}, 1)}
	f.ts = httptest.NewServer(http.HandlerFunc(f.serve))
	t.Cleanup(f.ts.Close)
	return f
}

func (f *fakeCore) dial(context.Context) (Core, error) { return HTTPCore(f.ts.URL, fakeToken), nil }

// forget drops every secret, as a restarted Core would.
func (f *fakeCore) forget() {
	f.mu.Lock()
	f.secrets = map[string]string{}
	f.mu.Unlock()
}

func (f *fakeCore) snapshot() ([]fakeCall, []fakeJoin) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]fakeCall(nil), f.calls...), append([]fakeJoin(nil), f.joins...)
}

func writeJSON(w http.ResponseWriter, code int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	_ = json.NewEncoder(w).Encode(v)
}

func (f *fakeCore) serve(w http.ResponseWriter, r *http.Request) {
	if r.Header.Get("Authorization") != "Bearer "+fakeToken {
		writeJSON(w, 401, map[string]string{"error": "bad token"})
		return
	}
	member := r.Header.Get(attach.HeaderMember)
	f.mu.Lock()
	_, known := f.secrets[member]
	f.mu.Unlock()
	switch {
	case r.Method == "GET" && r.URL.Path == "/v1/tools":
		if r.URL.Query().Get("client") != "grok" {
			writeJSON(w, 400, map[string]string{"error": "client"})
			return
		}
		f.mu.Lock()
		specs := f.specs
		f.mu.Unlock()
		if specs == nil {
			writeJSON(w, 404, map[string]string{"error": "not found"})
			return
		}
		writeJSON(w, 200, specs)
	case r.Method == "POST" && strings.HasPrefix(r.URL.Path, "/v1/tools/"):
		body, _ := io.ReadAll(r.Body)
		name := strings.TrimPrefix(r.URL.Path, "/v1/tools/")
		f.mu.Lock()
		f.calls = append(f.calls, fakeCall{Name: name, Member: member, Body: string(body)})
		f.mu.Unlock()
		if member != "" && !known {
			writeJSON(w, 403, map[string]string{"error": "unknown member"})
			return
		}
		writeJSON(w, 200, map[string]string{"text": "ok " + name})
	case r.Method == "POST" && r.URL.Path == "/v1/members":
		var in struct{ Name, Session string }
		if err := json.NewDecoder(r.Body).Decode(&in); err != nil {
			writeJSON(w, 400, map[string]string{"error": err.Error()})
			return
		}
		f.mu.Lock()
		f.joins = append(f.joins, fakeJoin(in))
		f.n++
		key, ok := f.keys[in.Session]
		if !ok {
			key = fmt.Sprintf("dev1/grok-%d", f.n)
			f.keys[in.Session] = key
		}
		secret := fmt.Sprintf("secret-%d", f.n)
		f.secrets[secret] = key
		f.mu.Unlock()
		writeJSON(w, 200, map[string]string{"memberKey": key, "memberSecret": secret, "name": in.Name})
	case r.Method == "GET" && r.URL.Path == "/v1/attach":
		secret := r.URL.Query().Get("member")
		f.mu.Lock()
		key, ok := f.secrets[secret]
		f.attaches = append(f.attaches, secret)
		f.mu.Unlock()
		if !ok {
			writeJSON(w, 403, map[string]string{"error": "unknown member"})
			return
		}
		w.Header().Set("Content-Type", "application/x-ndjson")
		rc := http.NewResponseController(w)
		enc := json.NewEncoder(w)
		_ = enc.Encode(attach.Event{Type: attach.TypeHello, V: attach.Version, AgentKey: key})
		_ = rc.Flush()
		f.streamOn <- secret
		for {
			select {
			case <-r.Context().Done():
				return
			case <-f.cut:
				return
			case ev := <-f.events:
				if _, err := io.WriteString(w, "\n"); err != nil { // a keepalive first
					return
				}
				if err := enc.Encode(ev); err != nil {
					return
				}
				_ = rc.Flush()
			}
		}
	default:
		writeJSON(w, 404, map[string]string{"error": "no route"})
	}
}

// tap is a client transport that pulls channel pushes off the wire before
// the SDK client sees them.
type tap struct {
	inner  mcp.Transport
	pushes chan *jsonrpc.Request
}

func (t *tap) Connect(ctx context.Context) (mcp.Connection, error) {
	c, err := t.inner.Connect(ctx)
	if err != nil {
		return nil, err
	}
	return &tapConn{Connection: c, pushes: t.pushes}, nil
}

type tapConn struct {
	mcp.Connection
	pushes chan *jsonrpc.Request
}

func (c *tapConn) Read(ctx context.Context) (jsonrpc.Message, error) {
	for {
		msg, err := c.Connection.Read(ctx)
		if err != nil {
			return nil, err
		}
		if req, ok := msg.(*jsonrpc.Request); ok && req.Method == ChannelMethod {
			c.pushes <- req
			continue
		}
		return msg, nil
	}
}

type harness struct {
	cs     *mcp.ClientSession
	pushes chan *jsonrpc.Request
	srv    *Server
	done   chan error
	cancel context.CancelFunc
}

// start runs srv on an in-memory transport and connects a client.
func start(t *testing.T, srv *Server, protocol string) *harness {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	st, ct := mcp.NewInMemoryTransports()
	h := &harness{pushes: make(chan *jsonrpc.Request, 16), srv: srv, done: make(chan error, 1), cancel: cancel}
	go func() { h.done <- srv.Run(ctx, st) }()
	client := mcp.NewClient(&mcp.Implementation{Name: "test-client", Version: "0"}, nil)
	cs, err := client.Connect(ctx, &tap{inner: ct, pushes: h.pushes}, &mcp.ClientSessionOptions{ProtocolVersion: protocol})
	if err != nil {
		cancel()
		t.Fatal(err)
	}
	h.cs = cs
	t.Cleanup(func() {
		cs.Close()
		cancel()
		select {
		case <-h.done:
		case <-time.After(5 * time.Second):
			t.Error("Run didn't return")
		}
	})
	return h
}

func (h *harness) call(t *testing.T, name string, args any, meta map[string]any) (string, bool) {
	t.Helper()
	p := &mcp.CallToolParams{Name: name, Arguments: args}
	if meta != nil {
		p.Meta = meta
	}
	res, err := h.cs.CallTool(context.Background(), p)
	if err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	if len(res.Content) != 1 {
		t.Fatalf("%s: %d content blocks", name, len(res.Content))
	}
	tc, ok := res.Content[0].(*mcp.TextContent)
	if !ok {
		t.Fatalf("%s: content %T", name, res.Content[0])
	}
	return tc.Text, res.IsError
}

func (h *harness) toolNames(t *testing.T) []string {
	t.Helper()
	list, err := h.cs.ListTools(context.Background(), nil)
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, tl := range list.Tools {
		names = append(names, tl.Name)
	}
	return names
}

func (h *harness) push(t *testing.T) *jsonrpc.Request {
	t.Helper()
	select {
	case p := <-h.pushes:
		return p
	case <-time.After(5 * time.Second):
		t.Fatal("no channel push")
		return nil
	}
}

func (h *harness) noPush(t *testing.T, d time.Duration) {
	t.Helper()
	select {
	case p := <-h.pushes:
		t.Fatalf("unexpected push %s", p.Params)
	case <-time.After(d):
	}
}

func waitStream(t *testing.T, f *fakeCore) string {
	t.Helper()
	select {
	case s := <-f.streamOn:
		return s
	case <-time.After(5 * time.Second):
		t.Fatal("member stream never opened")
		return ""
	}
}

package mcpserver

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/api"
	"github.com/StephenSHorton/irori/internal/attach"
	"github.com/StephenSHorton/irori/internal/tools"
)

// Core is the slice of the Core's loopback API (internal/api/routes_v02.go)
// that `irori mcp` uses. HTTPCore implements it; tests may fake it.
type Core interface {
	// Tools lists the tool specs for a client kind (GET /v1/tools?client=).
	Tools(ctx context.Context, client string) ([]tools.Spec, error)
	// CallTool runs one tool (POST /v1/tools/{name} {"args"}). member is the
	// member secret from Join, sent as X-Irori-Member; "" sends no identity.
	CallTool(ctx context.Context, member, name string, args json.RawMessage) (string, error)
	// Join binds a Grok member (POST /v1/members).
	Join(ctx context.Context, name, session string) (Member, error)
	// Attach opens the member's NDJSON event stream (GET /v1/attach?member=).
	// The caller closes it; cancelling ctx also ends it.
	Attach(ctx context.Context, memberSecret string) (io.ReadCloser, error)
}

// Member is POST /v1/members' answer.
type Member struct {
	Key    string `json:"memberKey"`
	Secret string `json:"memberSecret"`
	Name   string `json:"name"`
}

// maxCoreBody caps a JSON answer read from the Core.
const maxCoreBody = 4 << 20

// LoopbackURL is the API base for a lockfile port.
func LoopbackURL(port int) string { return "http://127.0.0.1:" + strconv.Itoa(port) }

// HTTPCore is Core over the loopback API at base (http://127.0.0.1:<port>)
// with the bearer token from the lockfile.
func HTTPCore(base, token string) Core {
	return &httpCore{
		base:  strings.TrimRight(base, "/"),
		token: token,
		http: &http.Client{
			Transport: &http.Transport{
				Proxy:           nil, // never route loopback through HTTP(S)_PROXY
				DialContext:     (&net.Dialer{Timeout: dialTimeout}).DialContext,
				MaxIdleConns:    4,
				IdleConnTimeout: 30 * time.Second,
			},
			// No client Timeout: the attach stream is long-lived. Calls get
			// their own deadline from callTimeout.
			CheckRedirect: func(*http.Request, []*http.Request) error {
				return http.ErrUseLastResponse // the token must not follow a redirect
			},
		},
	}
}

// callTimeout bounds one non-streaming request.
const callTimeout = 30 * time.Second

type httpCore struct {
	base  string
	token string
	http  *http.Client
}

// runID names the Core run this client talks to. The Core mints a new token
// on every start, so base+token changes exactly when the Core restarts.
func (c *httpCore) runID() string { return c.base + "|" + c.token }

// runOf is c's run identity: runID when c has one, else c itself (a fake).
func runOf(c Core) any {
	if r, ok := c.(interface{ runID() string }); ok {
		return r.runID()
	}
	return c
}

func (c *httpCore) Tools(ctx context.Context, client string) ([]tools.Spec, error) {
	var specs []tools.Spec
	err := c.do(ctx, http.MethodGet, "/v1/tools", url.Values{"client": {client}}, "", nil, &specs)
	return specs, err
}

func (c *httpCore) CallTool(ctx context.Context, member, name string, args json.RawMessage) (string, error) {
	if len(bytes.TrimSpace(args)) == 0 {
		args = json.RawMessage(`{}`)
	}
	var out struct {
		Text string `json:"text"`
	}
	err := c.do(ctx, http.MethodPost, "/v1/tools/"+url.PathEscape(name), nil, member,
		struct {
			Args json.RawMessage `json:"args"`
		}{args}, &out)
	return out.Text, err
}

func (c *httpCore) Join(ctx context.Context, name, session string) (Member, error) {
	var m Member
	err := c.do(ctx, http.MethodPost, "/v1/members", nil, "", struct {
		Name    string `json:"name"`
		Session string `json:"session,omitempty"`
	}{name, session}, &m)
	if err == nil && m.Secret == "" {
		err = errors.New("irori api /v1/members: answer has no memberSecret")
	}
	return m, err
}

func (c *httpCore) Attach(ctx context.Context, memberSecret string) (io.ReadCloser, error) {
	req, err := c.request(ctx, http.MethodGet, "/v1/attach", url.Values{"member": {memberSecret}}, "", nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Accept", "application/x-ndjson")
	resp, err := c.http.Do(req)
	if err != nil {
		// A *url.Error quotes the whole URL, query (the member secret)
		// included, and this error gets logged. Keep only its cause, which
		// still unwraps to the *net.OpError / context error.
		var uerr *url.Error
		if errors.As(err, &uerr) {
			err = uerr.Err
		}
		return nil, fmt.Errorf("irori api /v1/attach: %w", err)
	}
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		defer resp.Body.Close()
		return nil, statusError("/v1/attach", resp)
	}
	return resp.Body, nil
}

func (c *httpCore) request(ctx context.Context, method, path string, q url.Values, member string, body any) (*http.Request, error) {
	u := c.base + path
	if len(q) > 0 {
		u += "?" + q.Encode()
	}
	var rd io.Reader
	if body != nil {
		buf, err := json.Marshal(body)
		if err != nil {
			return nil, err
		}
		rd = bytes.NewReader(buf)
	}
	req, err := http.NewRequestWithContext(ctx, method, u, rd)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Bearer "+c.token)
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if member != "" {
		req.Header.Set(attach.HeaderMember, member)
	}
	return req, nil
}

func (c *httpCore) do(ctx context.Context, method, path string, q url.Values, member string, body, out any) error {
	ctx, cancel := context.WithTimeout(ctx, callTimeout)
	defer cancel()
	req, err := c.request(ctx, method, path, q, member, body)
	if err != nil {
		return err
	}
	req.Header.Set("Accept", "application/json")
	resp, err := c.http.Do(req)
	if err != nil {
		return fmt.Errorf("irori api %s: %w", path, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return statusError(path, resp)
	}
	data, err := io.ReadAll(io.LimitReader(resp.Body, maxCoreBody))
	if err != nil {
		return fmt.Errorf("irori api %s: %w", path, err)
	}
	if err := json.Unmarshal(data, out); err != nil {
		return fmt.Errorf("irori api %s: bad response: %w", path, err)
	}
	return nil
}

func statusError(path string, resp *http.Response) error {
	data, _ := io.ReadAll(io.LimitReader(resp.Body, 64<<10))
	var eb struct {
		Error string `json:"error"`
	}
	_ = json.Unmarshal(data, &eb)
	return &api.StatusError{Path: path, Code: resp.StatusCode, Message: eb.Error}
}

// statusCode is the HTTP status of a Core answer, or 0 when err isn't one
// (transport failures, timeouts).
func statusCode(err error) int {
	var se *api.StatusError
	if errors.As(err, &se) {
		return se.Code
	}
	return 0
}

// unsent reports whether err means the request never reached a server, so
// even a mutating call is safe to retry on a fresh dial.
func unsent(err error) bool {
	if errors.Is(err, ErrNotRunning) {
		return true
	}
	var op *net.OpError
	return errors.As(err, &op) && op.Op == "dial"
}

package api

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"

	"github.com/StephenSHorton/irori/internal/attach"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/tools"
)

// Identity is the caller header a v0.2 client call carries: exactly one of
// an attach secret (mod), a member secret (Grok) or the person's per-run
// secret (the GUI's, v0.4 D10(a)). None reads as an unidentified agent.
type Identity struct {
	Attach string
	Member string
	Person string
}

func (id Identity) set(req *http.Request) {
	if id.Person != "" {
		req.Header.Set(HeaderPerson, id.Person)
	}
	if id.Attach != "" {
		req.Header.Set(attach.HeaderAttach, id.Attach)
	}
	if id.Member != "" {
		req.Header.Set(attach.HeaderMember, id.Member)
	}
}

// Join binds a Grok member (POST /v1/members).
func (c *Client) Join(ctx context.Context, name, session string) (Member, error) {
	var m Member
	err := c.impl.do(ctx, http.MethodPost, "/v1/members", nil, Identity{}, nil,
		map[string]string{"name": name, "session": session}, &m)
	return m, err
}

// ToolSpecs lists the tools for client ("claude" or "grok").
func (c *Client) ToolSpecs(ctx context.Context, client string) ([]tools.Spec, error) {
	var specs []tools.Spec
	err := c.impl.get(ctx, "/v1/tools", url.Values{"client": {client}}, true, &specs)
	return specs, err
}

// CallTool runs one tool as id. A tool-level failure is a nil error and text
// starting "Error: ".
func (c *Client) CallTool(ctx context.Context, id Identity, name string, args json.RawMessage) (string, error) {
	if len(args) == 0 {
		args = json.RawMessage(`{}`)
	}
	var out ToolResult
	err := c.impl.do(ctx, http.MethodPost, "/v1/tools/"+url.PathEscape(name), nil, id, nil,
		map[string]json.RawMessage{"args": args}, &out)
	return out.Text, err
}

// Inbox lists the caller's deliveries (GET /v1/inbox).
func (c *Client) Inbox(ctx context.Context, id Identity, all bool, limit int) ([]inbox.Delivery, error) {
	q := url.Values{}
	if all {
		q.Set("all", "1")
	}
	if limit > 0 {
		q.Set("limit", fmt.Sprint(limit))
	}
	var out []inbox.Delivery
	err := c.impl.do(ctx, http.MethodGet, "/v1/inbox", q, id, nil, nil, &out)
	return out, err
}

// Policy reads the wake policy.
func (c *Client) Policy(ctx context.Context) (inbox.Policy, error) {
	var p inbox.Policy
	err := c.impl.get(ctx, "/v1/policy", nil, true, &p)
	return p, err
}

// Control posts one of the mod's delivery confirmations: what is
// "delivered", "engaged" or "turn-ended".
func (c *Client) Control(ctx context.Context, attachSecret, what, token, turnID string) error {
	var ok okBody
	return c.impl.do(ctx, http.MethodPost, "/v1/deliveries/"+what, nil, Identity{Attach: attachSecret}, nil,
		map[string]string{"token": token, "turnId": turnID}, &ok)
}

// OpenStream opens GET /v1/attach with q (session=&mod= or member=) and
// returns the NDJSON body. It has no overall timeout; cancel ctx to end it.
func (c *Client) OpenStream(ctx context.Context, q url.Values) (io.ReadCloser, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, c.impl.base+"/v1/attach?"+q.Encode(), nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Bearer "+c.impl.lf.Token)
	req.Header.Set("Accept", "application/x-ndjson")
	hc := *c.impl.http
	hc.Timeout = 0 // a stream lives as long as ctx
	resp, err := hc.Do(req)
	if err != nil {
		return nil, fmt.Errorf("irori api /v1/attach: %w", withoutQuery(err))
	}
	if resp.StatusCode != http.StatusOK {
		defer resp.Body.Close()
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 64<<10))
		var eb errorBody
		_ = json.Unmarshal(body, &eb)
		return nil, &StatusError{Path: "/v1/attach", Code: resp.StatusCode, Message: eb.Error}
	}
	return resp.Body, nil
}

// withoutQuery drops the query from a *url.Error's URL: a member stream's
// query carries the member secret, which must not reach error text or logs.
func withoutQuery(err error) error {
	var ue *url.Error
	if !errors.As(err, &ue) {
		return err
	}
	u, perr := url.Parse(ue.URL)
	if perr != nil {
		return ue.Err
	}
	u.RawQuery, u.ForceQuery = "", false
	return &url.Error{Op: ue.Op, URL: u.String(), Err: ue.Err}
}

// do sends an authenticated request with an optional JSON body.
func (c *client) do(ctx context.Context, method, path string, q url.Values, id Identity, hdr http.Header, in, out any) error {
	u := c.base + path
	if len(q) > 0 {
		u += "?" + q.Encode()
	}
	var body io.Reader
	if in != nil {
		raw, err := json.Marshal(in)
		if err != nil {
			return err
		}
		body = bytes.NewReader(raw)
	}
	req, err := http.NewRequestWithContext(ctx, method, u, body)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+c.lf.Token)
	req.Header.Set("Accept", "application/json")
	if in != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	id.set(req)
	for k, vs := range hdr {
		for _, v := range vs {
			req.Header.Add(k, v)
		}
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return fmt.Errorf("irori api %s: %w", path, err)
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, maxBody))
	if err != nil {
		return fmt.Errorf("irori api %s: %w", path, err)
	}
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		var eb errorBody
		_ = json.Unmarshal(raw, &eb)
		return &StatusError{Path: path, Code: resp.StatusCode, Message: eb.Error}
	}
	if out == nil {
		return nil
	}
	if err := json.Unmarshal(raw, out); err != nil {
		return fmt.Errorf("irori api %s: bad response: %w", path, err)
	}
	return nil
}

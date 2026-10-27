package api

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"time"

	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// probeTimeout bounds Dial's health check.
const probeTimeout = 2 * time.Second

// maxBody caps a response the client will read (1000 events with payloads).
const maxBody = 32 << 20

// StatusError is a non-2xx answer from the server.
type StatusError struct {
	Path    string
	Code    int
	Message string // the server's {"error"} text, if any
}

func (e *StatusError) Error() string {
	msg := e.Message
	if msg == "" {
		msg = http.StatusText(e.Code)
	}
	return fmt.Sprintf("irori api %s: %d %s", e.Path, e.Code, msg)
}

type client struct {
	lf   discovery.Lockfile
	base string
	http *http.Client
}

func newClient(lf discovery.Lockfile) *Client {
	return &Client{impl: &client{
		lf:   lf,
		base: "http://127.0.0.1:" + strconv.Itoa(lf.Port),
		http: &http.Client{
			// Never route loopback calls through an HTTP(S)_PROXY.
			Transport: &http.Transport{
				Proxy:           nil,
				DialContext:     (&net.Dialer{Timeout: probeTimeout}).DialContext,
				MaxIdleConns:    4,
				IdleConnTimeout: 30 * time.Second,
			},
			Timeout: 30 * time.Second,
			CheckRedirect: func(*http.Request, []*http.Request) error {
				return http.ErrUseLastResponse // the token must not follow a redirect
			},
		},
	}}
}

func dial(ctx context.Context, dataDir string) (*Client, error) {
	lf, err := discovery.Read(dataDir)
	if err != nil {
		return nil, err
	}
	c := newClient(lf)
	pctx, cancel := context.WithTimeout(ctx, probeTimeout)
	defer cancel()
	h, err := c.Health(pctx)
	if err != nil {
		var se *StatusError
		if errors.As(err, &se) {
			return nil, err
		}
		// The pid is alive but nothing answers on the port: treat it as gone.
		return nil, fmt.Errorf("%w: no answer on port %d: %v", discovery.ErrNotRunning, lf.Port, err)
	}
	if !h.OK || (lf.DeviceID != "" && h.DeviceID != lf.DeviceID) {
		return nil, fmt.Errorf("%w: port %d is not the irori in %s", discovery.ErrNotRunning, lf.Port, dataDir)
	}
	return c, nil
}

// Lockfile is the lockfile this client was built from.
func (c *Client) Lockfile() discovery.Lockfile { return c.impl.lf }

// Health calls GET /v1/health (no token needed).
func (c *Client) Health(ctx context.Context) (Health, error) {
	var h Health
	err := c.impl.get(ctx, "/v1/health", nil, false, &h)
	return h, err
}

func (c *Client) device(ctx context.Context) (device.Identity, error) {
	var id device.Identity
	err := c.impl.get(ctx, "/v1/device", nil, true, &id)
	return id, err
}

func (c *Client) agents(ctx context.Context) ([]presence.AgentCard, error) {
	var cards []presence.AgentCard
	err := c.impl.get(ctx, "/v1/agents", nil, true, &cards)
	return cards, err
}

func (c *Client) events(ctx context.Context, kinds []string, limit int) ([]store.Event, error) {
	q := url.Values{}
	for _, k := range kinds {
		q.Add("kind", k)
	}
	if limit > 0 {
		q.Set("limit", strconv.Itoa(limit))
	}
	var evs []store.Event
	err := c.impl.get(ctx, "/v1/events", q, true, &evs)
	return evs, err
}

func (c *Client) lastSeq(ctx context.Context, deviceID string) (int64, error) {
	if deviceID == "" {
		return 0, errors.New("irori api: LastSeq needs a device id")
	}
	// A device's own rows are HLC-ordered by seq, so its newest row has the
	// top seq.
	q := url.Values{"device": {deviceID}, "limit": {"1"}}
	var evs []store.Event
	if err := c.impl.get(ctx, "/v1/events", q, true, &evs); err != nil {
		return 0, err
	}
	if len(evs) == 0 {
		return 0, nil
	}
	if evs[0].DeviceID != deviceID {
		return 0, errors.New("irori api: this Core does not filter events by device; update it")
	}
	return evs[0].Seq, nil
}

func (c *client) get(ctx context.Context, path string, q url.Values, auth bool, out any) error {
	u := c.base + path
	if len(q) > 0 {
		u += "?" + q.Encode()
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, u, nil)
	if err != nil {
		return err
	}
	if auth {
		req.Header.Set("Authorization", "Bearer "+c.lf.Token)
	}
	req.Header.Set("Accept", "application/json")
	resp, err := c.http.Do(req)
	if err != nil {
		return fmt.Errorf("irori api %s: %w", path, err)
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, maxBody))
	if err != nil {
		return fmt.Errorf("irori api %s: %w", path, err)
	}
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		var eb errorBody
		_ = json.Unmarshal(body, &eb)
		return &StatusError{Path: path, Code: resp.StatusCode, Message: eb.Error}
	}
	if err := json.Unmarshal(body, out); err != nil {
		return fmt.Errorf("irori api %s: bad response: %w", path, err)
	}
	return nil
}

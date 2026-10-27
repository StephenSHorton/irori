// Package api is Irori's loopback HTTP API, used by `irori mcp`, `irori attach`,
// the Claude Code mod and suzuri. The Wails UI calls Go directly instead.
//
// Security: listen on 127.0.0.1 only; every route except GET /v1/health needs
// "Authorization: Bearer <token>" (constant-time compare); reject a Host header
// that isn't 127.0.0.1:<port> or localhost:<port> (DNS rebinding) and any
// request carrying an Origin header (browsers). Responses are JSON.
//
// Routes (v0.1):
//
//	GET /v1/health                -> {"ok":true,"version","deviceId"}   (no token)
//	GET /v1/device                -> device.Identity
//	GET /v1/agents                -> []presence.AgentCard
//	GET /v1/events?kind=&limit=   -> []store.Event (kind may repeat; prefix "agent.")
//	GET /v1/events/stream         -> NDJSON of store.Event (routes_events.go)
//
// CONTRACT (implemented by the discovery/api work package; keep these signatures):
package api

import (
	"context"
	"errors"
	"net"

	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
	"github.com/StephenSHorton/irori/internal/tools"
)

// ErrUnimplemented is returned by skeleton stubs.
var ErrUnimplemented = errors.New("api: not implemented yet")

// Backend is what the server serves; core.Core implements it.
type Backend interface {
	Version() string
	Device() device.Identity
	Agents() []presence.AgentCard
	Events(ctx context.Context, f store.Filter) ([]store.Event, error)
	// SubscribeEvents is store.Subscribe: rows appended or inserted after the
	// call until ctx ends; a zero Event (Seq 0) means rows were dropped and
	// the reader should re-read. It feeds GET /v1/events/stream.
	SubscribeEvents(ctx context.Context) <-chan store.Event

	// v0.2 (routes_v02.go). A backend without messaging returns nil from
	// Inbox/Tools; the v0.2 routes then answer 503.
	Inbox() *inbox.Inbox
	Coord() *coord.Coord
	Tools() *tools.Set
	// JoinMember binds a Grok member (POST /v1/members). The same non-empty
	// session gets the same Key; every join mints a new Secret and the old
	// one stops working.
	JoinMember(ctx context.Context, name, session string) (Member, error)
	// MemberBySecret maps X-Irori-Member (or ?member=) to its member.
	MemberBySecret(secret string) (Member, bool)
	// MemberStream reports that key's member stream opened (true) or closed
	// (false). The member's synthetic card stays listed while a stream is
	// open and for inbox.ReattachGrace after.
	MemberStream(key string, open bool)

	// v0.3 (routes_v03.go). nil when capture is off; the v0.3 routes then
	// answer 503.
	Capture() *capture.Capture
}

// Member is a bound Grok member.
type Member struct {
	Key    string `json:"memberKey"` // "<device>/grok-<id>"
	Secret string `json:"memberSecret"`
	Name   string `json:"name"`
}

// Server serves Backend on a loopback listener.
type Server struct {
	impl *server
}

// New makes a server that checks token on every authenticated route.
func New(b Backend, token string) *Server { return newServer(b, token) }

// Serve serves on ln until ctx ends, then shuts down gracefully.
func (s *Server) Serve(ctx context.Context, ln net.Listener) error { return s.serve(ctx, ln) }

// Listen opens 127.0.0.1:0.
func Listen() (net.Listener, error) { return net.Listen("tcp", "127.0.0.1:0") }

// Client talks to a running Irori found through its lockfile.
type Client struct {
	impl *client
}

// Dial reads the lockfile in dataDir and checks /v1/health.
func Dial(ctx context.Context, dataDir string) (*Client, error) { return dial(ctx, dataDir) }

// NewClient builds a client from a lockfile without probing it.
func NewClient(lf discovery.Lockfile) *Client { return newClient(lf) }

func (c *Client) Device(ctx context.Context) (device.Identity, error)      { return c.device(ctx) }
func (c *Client) Agents(ctx context.Context) ([]presence.AgentCard, error) { return c.agents(ctx) }
func (c *Client) Events(ctx context.Context, kinds []string, limit int) ([]store.Event, error) {
	return c.events(ctx, kinds, limit)
}

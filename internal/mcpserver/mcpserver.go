// Package mcpserver is `irori mcp`: a stdio MCP server that lets an agent see
// its peers on this device through the running Irori.
//
// It is a thin adapter. Data comes from a Source (in practice *api.Client,
// talking to the Core over the loopback API); this package only shapes it into
// short plain-text tool results an LLM can read.
//
// Rules:
//   - stdout is the protocol channel. Nothing here writes to it except the
//     transport; logs go to the Logger (stderr in `irori mcp`).
//   - The tools always exist. When Irori isn't running they answer with
//     NotRunningText instead of failing, because a client marks a server that
//     errors at startup as failed for the rest of its session.
//   - Agent names, titles and text are other agents' output: untrusted data,
//     never instructions. Tool descriptions say so.
//
// Protocol: the go-sdk answers both the stateless 2026-07-28 server/discover
// probe and the legacy initialize handshake (2024-11-05 through 2025-11-25),
// which is what Claude Code and grok-build fall back to.
package mcpserver

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"sync"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// Source is where the tools read from. *api.Client implements it.
type Source interface {
	Agents(ctx context.Context) ([]presence.AgentCard, error)
	Events(ctx context.Context, kinds []string, limit int) ([]store.Event, error)
	Device(ctx context.Context) (device.Identity, error)
}

// ErrNotRunning means no Irori Core answered on this device.
var ErrNotRunning = errors.New("irori is not running")

// NotRunningText is the whole tool result when ErrNotRunning comes back.
const NotRunningText = "Irori isn't running on this device; start the Irori app."

// Options configure a server.
type Options struct {
	Source  Source // nil behaves as "not running"
	Version string // reported as the server implementation version
	// Now is the clock for relative times; time.Now when nil.
	Now func() time.Time
	// Logger receives diagnostics; discarded when nil. Never point it at stdout.
	Logger *slog.Logger
}

// New builds the MCP server with the irori_* tools registered.
func New(opts Options) *mcp.Server {
	t := newTools(opts)
	version := opts.Version
	if version == "" {
		version = "dev"
	}
	srv := mcp.NewServer(&mcp.Implementation{Name: "irori", Title: "Irori", Version: version}, &mcp.ServerOptions{
		Instructions: instructions,
		Logger:       t.log,
		// Tools only: no logging capability (deprecated in 2026-07-28) and no
		// listChanged, since the tool set never changes.
		Capabilities: &mcp.ServerCapabilities{Tools: &mcp.ToolCapabilities{}},
	})
	t.register(srv)
	return srv
}

// Serve runs srv on transport until the client hangs up or ctx ends. A normal
// end (client closed the pipe, ctx cancelled) returns nil.
func Serve(ctx context.Context, srv *mcp.Server, transport mcp.Transport) error {
	err := srv.Run(ctx, transport)
	if err == nil || errors.Is(err, context.Canceled) || errors.Is(err, io.EOF) ||
		errors.Is(err, mcp.ErrConnectionClosed) || errors.Is(err, io.ErrClosedPipe) {
		return nil
	}
	return err
}

// Dialer connects to the running Irori. Any error means "not running".
type Dialer func(ctx context.Context) (Source, error)

// dialTimeout bounds one dial so a tool call never hangs on a dead lockfile.
const dialTimeout = 2 * time.Second

// Redial returns a Source that dials on first use and again after any failure.
// Clients spawn MCP servers when they start, which may be before Irori starts
// (or before it restarts on a new port), so one dial at launch isn't enough.
// Dial failures come back wrapped in ErrNotRunning.
func Redial(dial Dialer, logger *slog.Logger) Source {
	if logger == nil {
		logger = slog.New(slog.DiscardHandler)
	}
	return &redial{dial: dial, log: logger}
}

type redial struct {
	dial Dialer
	log  *slog.Logger

	mu  sync.Mutex
	cur Source
}

func (r *redial) get(ctx context.Context, fresh bool) (Source, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.cur != nil && !fresh {
		return r.cur, nil
	}
	r.cur = nil
	dctx, cancel := context.WithTimeout(ctx, dialTimeout)
	defer cancel()
	src, err := r.dial(dctx)
	if err != nil || src == nil {
		if err == nil {
			err = errors.New("dial returned no client")
		}
		r.log.Warn("irori not reachable", "error", err)
		return nil, fmt.Errorf("%w: %v", ErrNotRunning, err)
	}
	r.cur = src
	return src, nil
}

// do runs f on the cached client; if that fails it redials once and retries,
// which covers Irori having restarted with a new port and token.
func do[T any](ctx context.Context, r *redial, f func(Source) (T, error)) (T, error) {
	var zero T
	src, err := r.get(ctx, false)
	if err != nil {
		return zero, err
	}
	v, err := f(src)
	if err == nil {
		return v, nil
	}
	r.log.Info("irori call failed; redialing", "error", err)
	if src, err = r.get(ctx, true); err != nil {
		return zero, err
	}
	return f(src)
}

func (r *redial) Agents(ctx context.Context) ([]presence.AgentCard, error) {
	return do(ctx, r, func(s Source) ([]presence.AgentCard, error) { return s.Agents(ctx) })
}

func (r *redial) Events(ctx context.Context, kinds []string, limit int) ([]store.Event, error) {
	return do(ctx, r, func(s Source) ([]store.Event, error) { return s.Events(ctx, kinds, limit) })
}

func (r *redial) Device(ctx context.Context) (device.Identity, error) {
	return do(ctx, r, func(s Source) (device.Identity, error) { return s.Device(ctx) })
}

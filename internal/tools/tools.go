// Package tools is the one implementation of Irori's agent tools
// (docs/design/v0.2-talk.md §8). The Core serves them at POST /v1/tools/{name}
// and lists them at GET /v1/tools; the mod registers them as mcp__irori__<name>
// and `irori mcp` as irori_<name>. Both front ends are thin proxies, so the
// names, schemas, descriptions and output format live only here.
//
// CONTRACT (implemented by the tools work package; keep these signatures):
package tools

import (
	"context"
	"encoding/json"
	"errors"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// MaxResult caps every tool result (about 2k tokens).
const MaxResult = 8 << 10

var ErrUnknownTool = errors.New("tools: no such tool")

// Spec describes one tool to a front end.
type Spec struct {
	Name        string          `json:"name"` // bare: "send", "reply", ...
	Description string          `json:"description"`
	InputSchema json.RawMessage `json:"inputSchema"` // JSON Schema object
	ReadOnly    bool            `json:"readOnly"`
	// Clients lists who gets it: "claude" (the mod), "grok" (irori mcp). join and
	// ack are grok-only.
	Clients []string `json:"clients"`
}

// Caller is who is calling, established by the transport (attach secret or
// member secret), never by the arguments.
type Caller struct {
	Sender inbox.Sender
	Path   inbox.Path
}

// Backend is what the tools act on; core.Core provides it.
type Backend interface {
	Agents() []presence.AgentCard
	Events(ctx context.Context, f store.Filter) ([]store.Event, error)
	Inbox() *inbox.Inbox
	Coord() *coord.Coord
}

// Set is the tool registry.
type Set struct {
	impl *set
}

// New builds the registry over b.
func New(b Backend) *Set { return newSet(b) }

// Specs lists tools for a client kind ("claude" or "grok").
func (s *Set) Specs(client string) []Spec { return s.specs(client) }

// Call runs one tool. The result is text for the model, at most MaxResult
// bytes; tool-level failures come back as text starting "Error: " with a nil
// error (models handle those better). A non-nil error means the call itself
// was malformed (unknown tool, args not an object).
func (s *Set) Call(ctx context.Context, c Caller, name string, args json.RawMessage) (string, error) {
	return s.call(ctx, c, name, args)
}

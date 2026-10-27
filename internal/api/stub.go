package api

import (
	"context"
	"net"

	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// Skeleton stubs. The discovery/api work package replaces this file.

type server struct{}
type client struct{}

func newServer(Backend, string) *Server                            { return &Server{impl: &server{}} }
func (s *Server) serve(ctx context.Context, ln net.Listener) error { <-ctx.Done(); return ln.Close() }
func dial(context.Context, string) (*Client, error)                { return nil, ErrUnimplemented }
func newClient(discovery.Lockfile) *Client                         { return &Client{impl: &client{}} }
func (c *Client) device(context.Context) (device.Identity, error) {
	return device.Identity{}, ErrUnimplemented
}
func (c *Client) agents(context.Context) ([]presence.AgentCard, error) {
	return nil, ErrUnimplemented
}
func (c *Client) events(context.Context, []string, int) ([]store.Event, error) {
	return nil, ErrUnimplemented
}

package main

import (
	"context"
	"flag"
	"log/slog"
	"os"
	"os/signal"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/StephenSHorton/irori/internal/api"
	"github.com/StephenSHorton/irori/internal/mcpserver"
	"github.com/StephenSHorton/irori/internal/paths"
)

// runMCP serves the stdio MCP server until the client closes the pipe.
//
// stdout is the protocol channel, so everything else goes to stderr. The
// Windows release is a -H windowsgui binary with no console; that's fine,
// because the client spawns it with pipes for stdin and stdout.
func runMCP(args []string) error {
	fs := flag.NewFlagSet("mcp", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	if err := fs.Parse(args); err != nil {
		return err
	}
	logger := slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: slog.LevelWarn}))
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()

	// Dial on demand: Irori may start (or restart on a new port) after the
	// client has spawned us.
	src := mcpserver.Redial(func(ctx context.Context) (mcpserver.Source, error) {
		dir, err := paths.DataDir()
		if err != nil {
			return nil, err
		}
		c, err := api.Dial(ctx, dir)
		if err != nil {
			return nil, err // never wrap a nil *api.Client in the interface
		}
		return c, nil
	}, logger)
	srv := mcpserver.New(mcpserver.Options{Source: src, Version: version, Logger: logger})
	return mcpserver.Serve(ctx, srv, &mcp.StdioTransport{})
}

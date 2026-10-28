// Irori (囲炉裏) — a hearth for your coding agents.
//
// One binary, several roles:
//
//	irori                  the desktop app: tray, window, and the Core
//	irori serve --headless the Core with no UI (the always-on node)
//	irori mcp              stdio MCP server for agents, talking to the running Core
//	irori attach           the Claude Code mod's event stream (v0.2)
//	irori events           print or follow (--follow) the event log
//	irori pair join        join another device's person (add one from the app)
//	irori pair reset       make this device a new one (it can pair again)
//	irori node install     run serve --headless at logon (Windows Task Scheduler)
//	irori version
//
// The Core (internal/core) is the event log, presence and the loopback API.
// Exactly one runs per device; everything else finds it through the lockfile
// (internal/discovery).
package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"log"
	"os"
	"os/signal"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/paths"
)

// version is injected at release build time with -ldflags "-X main.version=X.Y.Z".
// "dev" builds never offer updates.
var version = "dev"

func main() {
	args := os.Args[1:]
	cmd := ""
	if len(args) > 0 && len(args[0]) > 0 && args[0][0] != '-' {
		cmd, args = args[0], args[1:]
	}
	useParentConsole(cmd)
	var err error
	switch cmd {
	case "", "gui":
		err = runGUI(args)
	case "serve":
		err = runServe(args)
	case "mcp":
		err = runMCP(args)
	case "attach":
		err = runAttach(args)
	case "events":
		err = runEvents(args)
	case "pair":
		err = runPair(args)
	case "node":
		err = runNode(args)
	case "version":
		fmt.Println("irori", version)
	default:
		err = fmt.Errorf("unknown command %q (gui, serve, mcp, attach, events, pair, node, version)", cmd)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "irori:", err)
		os.Exit(1)
	}
}

func coreOptions(headless bool) (core.Options, error) {
	data, err := paths.DataDir()
	if err != nil {
		return core.Options{}, err
	}
	claude, err := paths.ClaudeDir()
	if err != nil {
		return core.Options{}, err
	}
	return core.Options{DataDir: data, ClaudeDir: claude, Version: version, Headless: headless}, nil
}

// runServe runs the Core without a UI until interrupted.
func runServe(args []string) error {
	fs := flag.NewFlagSet("serve", flag.ContinueOnError)
	headless := fs.Bool("headless", false, "run without a UI (required for now)")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if !*headless {
		return errors.New("serve needs --headless (the desktop app is plain `irori`)")
	}
	opts, err := coreOptions(true)
	if err != nil {
		return err
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	c, err := core.Start(ctx, opts)
	if err != nil {
		return err
	}
	log.Printf("irori %s serving headless (device %s, data %s)", version, c.Device().ID, opts.DataDir)
	<-ctx.Done()
	return c.Close()
}

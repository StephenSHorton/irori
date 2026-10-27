package main

import (
	"context"
	"flag"
	"log"
	"os"
	"os/signal"

	"github.com/StephenSHorton/irori/internal/attach/runner"
	"github.com/StephenSHorton/irori/internal/paths"
)

// runAttach is `irori attach --session <id> [--cwd …] [--mod-version …]`, the
// Irori Claude Code mod's wake stream. stdout carries only event lines (NDJSON);
// logs go to stderr. The Windows release is a -H windowsgui binary: the mod
// spawns it with pipes, which work without a console, and none is allocated.
// It exits when stdin closes (the mod's child pipe closing means the session
// ended) or when a newer attach for the same session supersedes it.
func runAttach(args []string) error {
	fs := flag.NewFlagSet("attach", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	session := fs.String("session", "", "Claude Code session id (required)")
	cwd := fs.String("cwd", "", "the session's working directory (for logs)")
	modVersion := fs.String("mod-version", "", "the Irori mod's version")
	if err := fs.Parse(args); err != nil {
		return err
	}
	dir, err := paths.DataDir()
	if err != nil {
		return err
	}
	logger := log.New(os.Stderr, "", log.LstdFlags)
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	return runner.Run(ctx, runner.Options{
		Session: *session, Cwd: *cwd, ModVersion: *modVersion, DataDir: dir,
		Stdin: os.Stdin, Stdout: os.Stdout, Logf: logger.Printf,
	})
}

package main

import (
	"context"
	"errors"
	"flag"
	"log"
	"os"
	"os/signal"
	"time"

	"github.com/StephenSHorton/irori/internal/attach/runner"
	"github.com/StephenSHorton/irori/internal/paths"
	"github.com/StephenSHorton/irori/internal/proc"
)

// runAttach is `irori attach --session <id> [--cwd …] [--mod-version …]`, the
// Irori Claude Code mod's wake stream. stdout carries only event lines (NDJSON);
// logs go to stderr. The Windows release is a -H windowsgui binary: the mod
// spawns it with pipes, which work without a console, and none is allocated.
// It exits when the mod kills it (module unload, session end), when its parent
// process dies, or when a newer attach for the same session supersedes it.
// stdin is ignored: $.process.spawn starts children with stdin already closed.
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
	ctx, orphaned := context.WithCancel(ctx)
	defer orphaned()
	// Our parent started before we did; a process at its pid that started
	// later is a reuse.
	self, _ := proc.StartTime(os.Getpid())
	go watchParent(ctx, os.Getppid(), self, logger, orphaned)
	return runner.Run(ctx, runner.Options{
		Session: *session, Cwd: *cwd, ModVersion: *modVersion, DataDir: dir,
		Stdout: os.Stdout, Logf: logger.Printf,
	})
}

// watchParent cancels when the process that spawned us is gone, so an attach
// whose Claude Code died without killing it doesn't hold the session's stream.
// On Windows it waits on the parent's process handle and reacts the moment
// it exits; where it can't (another OS, no access) it checks every
// parentPollEvery. startedBy (our own start; zero if unknown) rules out a
// reused pid either way.
func watchParent(ctx context.Context, ppid int, startedBy time.Time, logger *log.Logger, cancel context.CancelFunc) {
	err := proc.WaitExit(ctx, ppid, startedBy)
	if ctx.Err() != nil {
		return
	}
	if errors.Is(err, proc.ErrCantWait) {
		pollParent(ctx, ppid, startedBy)
		if ctx.Err() != nil {
			return
		}
	}
	logger.Printf("attach: parent %d is gone; exiting", ppid)
	cancel()
}

// parentPollEvery is the fallback check's interval.
const parentPollEvery = 5 * time.Second

// pollParent returns when ppid is gone (or reused) or ctx ends.
func pollParent(ctx context.Context, ppid int, startedBy time.Time) {
	t := time.NewTicker(parentPollEvery)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			if !proc.Alive(ppid) {
				return
			}
			if st, ok := proc.StartTime(ppid); ok && !startedBy.IsZero() && st.After(startedBy) {
				return
			}
		}
	}
}

package main

import (
	"bufio"
	"context"
	"errors"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/pair"
)

// runPair is `irori pair` (add a device from this one) and `irori pair join
// <code>` (join another device's person). It starts its own Core, so it
// refuses while Irori runs here: pairing is never on the loopback API, and
// a running app pairs from Settings → Devices. This is the path for a
// machine without the desktop app (the headless node, design §7).
func runPair(args []string) error {
	if len(args) > 0 && args[0] == "join" {
		return runPairJoin(args[1:])
	}
	fs := flag.NewFlagSet("pair", flag.ContinueOnError)
	role := fs.String("role", identity.RoleDesktop, "the new device's role: desktop or node")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *role != identity.RoleDesktop && *role != identity.RoleNode {
		return fmt.Errorf("--role is desktop or node")
	}
	c, ctx, stop, err := pairCore()
	if err != nil {
		return err
	}
	defer stop()
	p := c.Pairing()
	st, err := p.Add(*role)
	if err != nil {
		return err
	}
	fmt.Printf("Pairing code: %s\n", st.Code)
	fmt.Println("Treat it like a one-use password: whoever has it can reach this device first.")
	fmt.Println("On the new device run `irori pair join <code>`, or use Settings → Devices → Join with a code.")
	fmt.Printf("It works for %d minutes.\n", int(time.Until(st.Expires).Round(time.Minute)/time.Minute))
	lines := stdinLines()
	asked := false
	for {
		st = p.Status()
		switch st.State {
		case pair.StateConfirm:
			if !asked && st.Joiner != nil {
				fmt.Printf("\n%s (%s) wants to join. Its key: %s\n", st.Joiner.Name, st.Joiner.OS, st.Joiner.Fingerprint)
				fmt.Print("Type the words it shows: ")
				asked = true
			}
			select {
			case line, ok := <-lines:
				if !ok {
					p.Cancel()
					return errors.New("no words typed")
				}
				if err := p.Confirm(ctx, line); errors.Is(err, pair.ErrWrongWords) {
					if p.Status().State == pair.StateConfirm {
						fmt.Print("Those aren't the words. Try once more: ")
					}
				} else if err != nil {
					return err
				}
				continue
			case <-ctx.Done():
				p.Cancel()
				return ctx.Err()
			case <-time.After(200 * time.Millisecond):
			}
		case pair.StateDone:
			fmt.Println("Paired. Run Irori here (or `irori serve --headless`) to keep the devices in sync.")
			return nil
		case pair.StateWaiting:
		default:
			return errors.New(st.Error)
		}
		select {
		case <-ctx.Done():
			p.Cancel()
			return ctx.Err()
		case <-time.After(200 * time.Millisecond):
		}
	}
}

func runPairJoin(args []string) error {
	if len(args) != 1 {
		return errors.New("usage: irori pair join <code>")
	}
	c, ctx, stop, err := pairCore()
	if err != nil {
		return err
	}
	defer stop()
	p := c.Pairing()
	if _, err := p.Join(args[0]); err != nil {
		return err
	}
	fmt.Println("Looking for the other device...")
	shown := false
	for {
		st := p.Status()
		switch st.State {
		case pair.StateWords:
			if !shown {
				who := st.Peer
				if who == "" {
					who = "the other device"
				}
				fmt.Printf("Type these words on %s:\n\n    %s\n\nWaiting for %s (Ctrl+C cancels).\n", who, strings.Join(st.Words, " "), who)
				shown = true
			}
		case pair.StateDone:
			fmt.Println("Paired. Run Irori here (or `irori serve --headless`) to keep the devices in sync.")
			return nil
		case pair.StateWaiting:
		default:
			return errors.New(st.Error)
		}
		select {
		case <-ctx.Done():
			p.Cancel()
			return ctx.Err()
		case <-time.After(200 * time.Millisecond):
		}
	}
}

// pairCore starts a headless Core for the pairing commands.
func pairCore() (*core.Core, context.Context, func(), error) {
	opts, err := coreOptions(true)
	if err != nil {
		return nil, nil, nil, err
	}
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt)
	c, err := core.Start(ctx, opts)
	if errors.Is(err, core.ErrAlreadyRunning) {
		cancel()
		return nil, nil, nil, errors.New("Irori is running on this device: pair from Settings → Devices in the app, or quit it first")
	}
	if err != nil {
		cancel()
		return nil, nil, nil, err
	}
	return c, ctx, func() { c.Close(); cancel() }, nil
}

// stdinLines reads lines the person types.
func stdinLines() <-chan string {
	out := make(chan string)
	go func() {
		defer close(out)
		sc := bufio.NewScanner(os.Stdin)
		for sc.Scan() {
			out <- sc.Text()
		}
	}()
	return out
}

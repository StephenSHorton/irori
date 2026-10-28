package main

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"os"
	"os/signal"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/mesh/roomnet"
	"github.com/StephenSHorton/irori/internal/pair"
)

// runPair is `irori pair join`: this device joins another device's person,
// for a machine without the desktop app (the headless node, design §7). It
// starts its own Core, so it refuses while Irori runs here.
//
// There is no Add side here (design §3: an agent with a shell can't pair a
// device). Adding a device hands it the person secret and every row, and is
// only in the desktop app's Settings → Devices. Joining is safe to offer:
// it needs the person to type this device's words into the other device's
// app, and the other device's key here. The code is read from a prompt,
// never from argv, so it stays out of shell history and process lists.
func runPair(args []string) error {
	if len(args) == 1 && args[0] == "reset" {
		return runPairReset()
	}
	if len(args) == 0 || args[0] != "join" {
		return errors.New("usage: irori pair join | irori pair reset (add a device from Settings → Devices in the desktop app)")
	}
	if len(args) > 1 {
		return errors.New("usage: irori pair join (it asks for the code; don't pass it as an argument)")
	}
	lines := stdinLines()
	fmt.Print("Pairing code (from Settings → Devices → Pair a device on the other device): ")
	code, ok := <-lines
	if !ok || strings.TrimSpace(code) == "" {
		return errors.New("no code typed")
	}
	// At home the other device is found on the local network; from away,
	// the home server's address (shown under the code) gets this device in.
	fmt.Print("Home server address, only when joining from outside your home (Enter to skip): ")
	server, _ := <-lines
	if server = strings.TrimSpace(server); server != "" {
		hp, err := roomnet.HostPort(server, 0)
		if err != nil {
			return errors.New("the home server address is a host and port, like 203.0.113.7:24817")
		}
		server = hp
	}
	c, ctx, stop, err := pairCore()
	if err != nil {
		return err
	}
	defer stop()
	p := c.Pairing()
	if _, err := p.JoinVia(code, server); err != nil {
		return err
	}
	fmt.Println("Looking for the other device...")
	shown := false
	for {
		st := p.Status()
		switch st.State {
		case pair.StateWords:
			who := st.Peer
			if who == "" {
				who = "the other device"
			}
			if !shown {
				fmt.Printf("Type these words on %s:\n\n    %s\n\n", who, strings.Join(st.Words, " "))
				fmt.Printf("Then type the key %s shows beside its code: ", who)
				shown = true
			}
			if st.KeyOK {
				break
			}
			select {
			case line, ok := <-lines:
				if !ok {
					p.Cancel()
					return errors.New("no key typed")
				}
				if err := p.ConfirmKey(ctx, line); errors.Is(err, pair.ErrWrongKey) {
					if p.Status().State == pair.StateWords {
						fmt.Print("That isn't the key. Try once more: ")
					}
				} else if err != nil {
					return err
				} else {
					fmt.Printf("Waiting for %s (Ctrl+C cancels).\n", who)
				}
				continue
			case <-ctx.Done():
				p.Cancel()
				return ctx.Err()
			case <-time.After(200 * time.Millisecond):
			}
			continue
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

// runPairReset is `irori pair reset` (design §3, Reset): this device's log,
// device id and keys move aside, and it starts next time as a new device
// that can pair again. Only the settings stay; nothing is deleted.
func runPairReset() error {
	opts, err := coreOptions(true)
	if err != nil {
		return err
	}
	fmt.Println("This makes this device a new one: its event log, device id and keys move aside, and only the settings stay.")
	fmt.Print("Type reset to go on: ")
	line, ok := <-stdinLines()
	if !ok || strings.TrimSpace(line) != "reset" {
		return errors.New("not reset")
	}
	dst, err := core.Reset(opts.DataDir)
	if err != nil {
		return err
	}
	fmt.Printf("Done. The old files are in %s. Start Irori to pair this device again.\n", dst)
	return nil
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

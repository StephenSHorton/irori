package main

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/StephenSHorton/irori/internal/api"
	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/mesh/roomnet"
	"github.com/StephenSHorton/irori/internal/pair"
)

// runPair is `irori pair join`: this device joins another device's person.
// While Irori runs here it drives that Core's join through the loopback API
// (api routes_pair.go); otherwise it starts its own Core, for a machine
// without the desktop app (the headless node, design §7).
//
// There is no Add side here (design §3: an agent with a shell can't pair a
// device). Adding a device hands it the person secret and every row, and is
// only in the desktop app's Settings → Devices. Joining is safe to offer:
// it needs the person to type this device's words into the other device's
// app, and the other device's key here. The code and the key are read from
// prompts, never from argv, so they stay out of shell history and process
// lists; stdin may be a pipe.
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
	opts, err := coreOptions(true)
	if err != nil {
		return err
	}
	lines := stdinLines()
	code, server, err := askJoin(lines, os.Stdout)
	if err != nil {
		return err
	}
	// Only from here: at the prompts above Ctrl+C ends the command as
	// usual, and once a join is open it closes that join on the way out.
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM, syscall.SIGHUP)
	defer stop()
	side, closeSide, err := openJoin(ctx, opts)
	if err != nil {
		return err
	}
	defer closeSide()
	return joinLoop(ctx, side, code, server, lines, os.Stdout, 300*time.Millisecond)
}

// askJoin prompts for the code and, from away, the home server address.
func askJoin(lines <-chan string, out io.Writer) (code, server string, err error) {
	fmt.Fprint(out, "Pairing code (from Settings → Devices → Pair a device on the other device): ")
	code, ok := <-lines
	if !ok || strings.TrimSpace(code) == "" {
		return "", "", errors.New("no code typed")
	}
	// At home the other device is found on the local network; from away,
	// the home server's address (shown under the code) gets this device in.
	fmt.Fprint(out, "Home server address, only when joining from outside your home (Enter to skip): ")
	server, _ = <-lines
	if server = strings.TrimSpace(server); server != "" {
		hp, err := roomnet.HostPort(server, 0)
		if err != nil {
			return "", "", errors.New("the home server address is a host and port, like 203.0.113.7:24817")
		}
		server = hp
	}
	return code, server, nil
}

// joinSide is the joining half of pairing: in the running Core through its
// API, or in a Core this command started.
type joinSide interface {
	Join(ctx context.Context, code, server string) (pair.Status, error)
	Status(ctx context.Context) (pair.Status, error)
	// Key types the other device's key: pair.ErrWrongKey when it isn't.
	Key(ctx context.Context, key string) error
	// Cancel closes the join, if it is still open.
	Cancel()
}

// openJoin picks the side: the running Core when there is one, else a Core
// of its own (today's path for a machine where Irori isn't running).
func openJoin(ctx context.Context, opts core.Options) (joinSide, func(), error) {
	c, err := api.Dial(ctx, opts.DataDir)
	if err == nil {
		return apiJoin{c}, func() {}, nil
	}
	// An Irori that answered with an error is running; anything else (no
	// lockfile, a dead pid, an unreadable irori.json) goes on to start a
	// Core, which refuses itself if one does run.
	var se *api.StatusError
	if errors.As(err, &se) {
		return nil, nil, err
	}
	pc, stop, err := pairCore(ctx, opts)
	if err != nil {
		return nil, nil, err
	}
	return localJoin{pc.Pairing()}, stop, nil
}

// joinLoop joins with code, prints the words for the person to type on the
// other device, takes the other device's key from lines and follows the
// pairing until it is done, failed or expired. Once its join is open, every
// way out but "Paired." closes it, so the running Core isn't left holding a
// join nobody follows (and refusing the app's own Add or Join as busy).
func joinLoop(ctx context.Context, side joinSide, code, server string, lines <-chan string, out io.Writer, every time.Duration) (err error) {
	first, err := side.Join(ctx, code, server)
	if err != nil {
		return err
	}
	ours := true
	defer func() {
		// Not when the status stopped being this join: that pairing is
		// someone else's (the app's), and it isn't this command's to close.
		if err != nil && ours {
			side.Cancel()
		}
	}()
	fmt.Fprintln(out, "Looking for the other device...")
	shown := false
	for {
		st, err := side.Status(ctx)
		if err != nil {
			if ctx.Err() != nil {
				return ctx.Err()
			}
			return err
		}
		// The running Core has one pairing at a time, shared with the app:
		// once this join ended, the status may be a newer Add or Join.
		if st.Mode != pair.ModeJoin || !st.Expires.Equal(first.Expires) {
			ours = false
			return errors.New("the pairing closed")
		}
		switch st.State {
		case pair.StateWords:
			who := st.Peer
			if who == "" {
				who = "the other device"
			}
			if !shown {
				fmt.Fprintf(out, "Type these words on %s:\n\n    %s\n\n", who, strings.Join(st.Words, " "))
				fmt.Fprintf(out, "Then type the key %s shows beside its code: ", who)
				shown = true
			}
			if st.KeyOK {
				break
			}
			select {
			case line, ok := <-lines:
				if !ok {
					return errors.New("no key typed")
				}
				if err := side.Key(ctx, line); errors.Is(err, pair.ErrWrongKey) {
					if st, err := side.Status(ctx); err == nil && st.State == pair.StateWords {
						fmt.Fprint(out, "That isn't the key. Try once more: ")
					}
				} else if err != nil {
					return err
				} else {
					fmt.Fprintf(out, "Waiting for %s (Ctrl+C cancels).\n", who)
				}
				continue
			case <-ctx.Done():
				return ctx.Err()
			case <-time.After(every):
			}
			continue
		case pair.StateDone:
			fmt.Fprintln(out, "Paired. Keep Irori running here (or `irori serve --headless`) to keep the devices in sync.")
			return nil
		case pair.StateWaiting:
		default:
			ours = false // already closed: nothing to cancel
			if st.Error == "" {
				return errors.New("the pairing closed")
			}
			return errors.New(st.Error)
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(every):
		}
	}
}

// localJoin is the join in a Core this command started.
type localJoin struct{ p *pair.Pairer }

func (l localJoin) Join(_ context.Context, code, server string) (pair.Status, error) {
	return l.p.JoinVia(code, server)
}
func (l localJoin) Status(context.Context) (pair.Status, error) { return l.p.Status(), nil }
func (l localJoin) Key(ctx context.Context, key string) error   { return l.p.ConfirmKey(ctx, key) }
func (l localJoin) Cancel()                                     { l.p.CancelJoin() }

// apiJoin is the join in the running Core, over its loopback API.
type apiJoin struct{ c *api.Client }

func (a apiJoin) Join(ctx context.Context, code, server string) (pair.Status, error) {
	st, err := a.c.PairJoin(ctx, code, server)
	if api.IsNoRoute(err) {
		return st, errors.New("the Irori running on this device is older than this command: update it, or quit it and run `irori pair join` again")
	}
	return st, apiText(err)
}

func (a apiJoin) Status(ctx context.Context) (pair.Status, error) {
	st, err := a.c.PairStatus(ctx)
	return st, apiText(err)
}

func (a apiJoin) Key(ctx context.Context, key string) error {
	_, err := a.c.PairKey(ctx, key)
	if api.IsWrongKey(err) {
		return pair.ErrWrongKey
	}
	return apiText(err)
}

// Cancel runs even after ctx ended (Ctrl+C), so it has its own deadline.
func (a apiJoin) Cancel() {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_, _ = a.c.PairCancel(ctx)
}

// apiText is the server's own message for a refused call (it never carries
// the code or the key), without the route and status around it.
func apiText(err error) error {
	var se *api.StatusError
	if errors.As(err, &se) && se.Message != "" {
		return errors.New(se.Message)
	}
	return err
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

// pairCore starts a headless Core for `irori pair join` when Irori isn't
// running here.
func pairCore(ctx context.Context, opts core.Options) (*core.Core, func(), error) {
	c, err := core.Start(ctx, opts)
	if errors.Is(err, core.ErrAlreadyRunning) {
		// It started between the API check and here, or it runs but its
		// API doesn't answer.
		return nil, nil, errors.New("Irori is running on this device but didn't answer: run `irori pair join` again, or quit Irori first")
	}
	if err != nil {
		return nil, nil, err
	}
	return c, func() { c.Close() }, nil
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

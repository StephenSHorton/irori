// Package runner is `irori attach`: it dials the running Core through the
// lockfile, opens GET /v1/attach?session=<id> and copies each event line to
// stdout unchanged, reconnecting with backoff until stdin closes or the
// stream says superseded (docs/design/v0.2-talk.md §7).
//
// stdout is the mod's channel: nothing but event lines is ever written to
// it. Diagnostics go to Options.Logf (stderr in the binary).
package runner

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"time"

	"github.com/StephenSHorton/irori/internal/api"
	"github.com/StephenSHorton/irori/internal/attach"
)

// Backoff bounds between reconnect attempts.
const (
	MinBackoff = time.Second
	MaxBackoff = 16 * time.Second
)

// maxLine caps one event line (a wake prompt is at most ~16 KB of text,
// JSON-escaped).
const maxLine = 1 << 20

// Options configure Run.
type Options struct {
	Session    string // Claude Code session id ($.session.id())
	Cwd        string // informational (logs)
	ModVersion string
	DataDir    string // where the lockfile is (paths.DataDir)

	Stdin  io.Reader // optional: closing it (EOF) ends Run. `irori attach` leaves it nil, because $.process.spawn closes a child's stdin from the start
	Stdout io.Writer // event lines only
	Logf   func(format string, args ...any)

	// Test overrides; zero means the constants above.
	MinBackoff, MaxBackoff time.Duration
}

// errSuperseded ends Run successfully.
var errSuperseded = errors.New("superseded")

// Run attaches until stdin closes (nil), the stream is superseded (nil) or
// stdout can't be written (an error).
func Run(ctx context.Context, o Options) error {
	if o.Session == "" {
		return errors.New("attach: --session is required")
	}
	if o.Logf == nil {
		o.Logf = func(string, ...any) {}
	}
	if o.MinBackoff <= 0 {
		o.MinBackoff = MinBackoff
	}
	if o.MaxBackoff <= 0 {
		o.MaxBackoff = MaxBackoff
	}
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	if o.Stdin != nil {
		go func() {
			_, _ = io.Copy(io.Discard, o.Stdin)
			o.Logf("attach: stdin closed; exiting")
			cancel()
		}()
	}
	out := &writer{w: o.Stdout}
	backoff := o.MinBackoff
	outage := false // an "Irori isn't running" error was already emitted
	for {
		connected, err := attachOnce(ctx, o, out)
		switch {
		case errors.Is(err, errSuperseded):
			o.Logf("attach: superseded by a newer attach for %s", o.Session)
			return nil
		case ctx.Err() != nil:
			return nil
		case errors.Is(err, errStdout):
			return err
		}
		if connected {
			backoff, outage = o.MinBackoff, false
		}
		var se *api.StatusError
		switch {
		case err == nil:
			o.Logf("attach: stream ended; reconnecting")
		case errors.As(err, &se) && se.Code == http.StatusNotFound:
			o.Logf("attach: session %s not on the roster yet; retrying", o.Session)
		case errors.As(err, &se):
			o.Logf("attach: %v", err)
		default:
			o.Logf("attach: %v", err)
			if !outage {
				outage = true
				if werr := out.event(attach.Event{Type: attach.TypeError,
					Message: "Irori isn't running, so messages can't reach this session. It attaches by itself once Irori starts."}); werr != nil {
					return werr
				}
			}
		}
		select {
		case <-ctx.Done():
			return nil
		case <-time.After(backoff):
		}
		backoff = min(backoff*2, o.MaxBackoff)
	}
}

// attachOnce dials and copies one stream. connected reports that a hello
// arrived (the outage, if any, is over). A nil error is a stream that ended.
func attachOnce(ctx context.Context, o Options, out *writer) (connected bool, err error) {
	c, err := api.Dial(ctx, o.DataDir)
	if err != nil {
		return false, err
	}
	q := url.Values{"session": {o.Session}}
	if o.ModVersion != "" {
		q.Set("mod", o.ModVersion)
	}
	body, err := c.OpenStream(ctx, q)
	if err != nil {
		var se *api.StatusError
		if errors.As(err, &se) {
			return false, err
		}
		return false, fmt.Errorf("%w (open stream)", err) // treated as an outage
	}
	defer body.Close()
	o.Logf("attach: connected to irori on port %d (session %s, cwd %q)", c.Lockfile().Port, o.Session, o.Cwd)
	r := bufio.NewReaderSize(body, 64<<10)
	for {
		line, err := readLine(r)
		if len(line) > 0 {
			var head struct {
				Type string `json:"type"`
			}
			if json.Unmarshal(line, &head) != nil {
				o.Logf("attach: skipping a line that isn't JSON (%d bytes)", len(line))
			} else {
				if werr := out.line(line); werr != nil {
					return connected, werr
				}
				switch head.Type {
				case attach.TypeHello:
					connected = true
				case attach.TypeSuperseded:
					return connected, errSuperseded
				}
			}
		}
		if err != nil {
			if errors.Is(err, io.EOF) || ctx.Err() != nil {
				return connected, nil
			}
			if connected {
				o.Logf("attach: stream: %v", err)
				return connected, nil
			}
			return connected, err
		}
	}
}

// readLine returns the next line without its newline, trimmed of spaces; a
// keepalive comes back empty.
func readLine(r *bufio.Reader) ([]byte, error) {
	var buf []byte
	for {
		chunk, err := r.ReadSlice('\n')
		if len(buf)+len(chunk) > maxLine {
			// Drop an oversized line whole.
			for errors.Is(err, bufio.ErrBufferFull) {
				_, err = r.ReadSlice('\n')
			}
			return nil, err
		}
		buf = append(buf, chunk...)
		if errors.Is(err, bufio.ErrBufferFull) {
			continue
		}
		return bytes.TrimSpace(buf), err
	}
}

var errStdout = errors.New("attach: stdout closed")

// writer writes whole lines to stdout.
type writer struct{ w io.Writer }

func (w *writer) line(p []byte) error {
	buf := make([]byte, 0, len(p)+1)
	buf = append(append(buf, p...), '\n')
	if _, err := w.w.Write(buf); err != nil {
		return fmt.Errorf("%w: %v", errStdout, err)
	}
	return nil
}

func (w *writer) event(ev attach.Event) error {
	raw, err := json.Marshal(ev)
	if err != nil {
		return err
	}
	return w.line(raw)
}

package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/signal"
	"slices"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/api"
	"github.com/StephenSHorton/irori/internal/paths"
	"github.com/StephenSHorton/irori/internal/store"
)

// Reconnect backoff for `irori events --follow` when the Core goes away.
const (
	followRetryMin = 1 * time.Second
	followRetryMax = 10 * time.Second
)

// runEvents is `irori events [--follow] [--kinds a,b] [--json] [--limit N]
// [--since SEQ]`: the event log of the running Core. Without --follow it
// prints the newest rows (oldest first) and exits. With --follow it prints
// them and then streams new rows as they are written (GET /v1/events/stream),
// with no polling; --since SEQ replays this device's rows after SEQ instead.
// A follow survives a Core restart (it redials and resumes after the last
// seq it printed). One line per row; --json prints the raw NDJSON rows, and
// {"resync":true} where rows may have been dropped.
func runEvents(args []string) error {
	fs := flag.NewFlagSet("events", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	follow := fs.Bool("follow", false, "keep streaming new rows")
	kindsFlag := fs.String("kinds", "", "comma list of kinds or prefixes ending in \".\" (agent.,device.started)")
	asJSON := fs.Bool("json", false, "print raw NDJSON rows")
	limit := fs.Int("limit", 20, "recent rows to print first (0 = none with --follow)")
	since := fs.Int64("since", -1, "with --follow: replay this device's rows after this seq instead of the recent rows")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() > 0 {
		return fmt.Errorf("events: unexpected argument %q", fs.Arg(0))
	}
	dir, err := paths.DataDir()
	if err != nil {
		return err
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	p := &eventPrinter{w: os.Stdout, json: *asJSON}
	kinds := splitKinds(*kindsFlag)

	c, err := api.Dial(ctx, dir)
	if err != nil {
		return err
	}
	self, err := c.Device(ctx)
	if err != nil {
		return err
	}
	p.self = self.ID
	if !*follow {
		return p.recent(ctx, c, kinds, *limit)
	}
	return followEvents(ctx, dir, c, kinds, *since, *limit, p)
}

// followEvents streams rows until ctx ends, redialling when the Core goes
// away and reopening after a resync, both resuming after p.lastSelf.
// since < 0 starts after this device's newest row (looked up first, so a
// resync before any own row is printed still replays what was dropped) and
// prints the newest recent rows once the first stream is open, so a row
// written meanwhile is in the list or the stream and printed once. since >=
// 0 replays from there and prints no recent rows.
func followEvents(ctx context.Context, dir string, c *api.Client, kinds []string, since int64, recent int, p *eventPrinter) error {
	if since < 0 {
		top, err := c.LastSeq(ctx, p.self)
		if err != nil {
			return err
		}
		since = top
	} else {
		recent = 0
	}
	p.lastSelf = max(p.lastSelf, since)
	opened := func() error { // runs once each stream is open
		if recent <= 0 {
			return nil
		}
		if err := p.recent(ctx, c, kinds, recent); err != nil {
			return err
		}
		recent = 0
		return nil
	}
	wait := followRetryMin
	idle := 0 // resyncs in a row that printed nothing and moved no seq
	for {
		if c != nil {
			from, printed := p.lastSelf, p.printed
			err := streamOnce(ctx, c, kinds, from, p, opened)
			if ctx.Err() != nil {
				return nil
			}
			var se *api.StatusError
			if errors.As(err, &se) {
				return err
			}
			if errors.Is(err, errOutput) {
				return err
			}
			if p.lastSelf != from || p.printed != printed {
				idle = 0
			}
			if errors.Is(err, errResync) {
				if idle++; idle == 1 {
					wait = followRetryMin
					continue // reopen now, resuming from the last seq
				}
				// Resynced again with nothing to show for it: back off
				// rather than spin on a row that cannot be read.
			} else {
				fmt.Fprintf(os.Stderr, "irori events: stream ended (%v); reconnecting\n", errOr(err, "Core closed it"))
				c, idle = nil, 0
			}
		}
		select {
		case <-ctx.Done():
			return nil
		case <-time.After(wait):
		}
		wait = min(wait*2, followRetryMax)
		if c != nil {
			continue
		}
		nc, err := api.Dial(ctx, dir)
		if err != nil {
			continue
		}
		c, wait = nc, followRetryMin
	}
}

var (
	errResync = errors.New("rows were dropped")
	errOutput = errors.New("events: stdout closed")
)

func errOr(err error, s string) string {
	if err != nil {
		return err.Error()
	}
	return s
}

// streamOnce runs one stream until it ends, calling opened once it is open.
// A resync is printed and returned as errResync, so the caller reopens with
// since; an own row skipped as too long counts as seen, so the reopened
// stream moves past it.
func streamOnce(ctx context.Context, c *api.Client, kinds []string, since int64, p *eventPrinter, opened func() error) error {
	sctx, cancel := context.WithCancel(ctx)
	defer cancel()
	ch, err := c.EventsStream(sctx, kinds, since)
	if err != nil {
		return err
	}
	if err := opened(); err != nil {
		return err
	}
	for it := range ch {
		switch {
		case it.Err != nil:
			return it.Err
		case it.Resync:
			if it.SkippedDevice == p.self && it.SkippedSeq > p.lastSelf {
				p.lastSelf = it.SkippedSeq
			}
			if p.resync() != nil {
				return errOutput
			}
			return errResync
		default:
			if p.event(it.Event) != nil {
				return errOutput
			}
		}
	}
	return nil
}

func splitKinds(s string) []string {
	var out []string
	for _, k := range strings.Split(s, ",") {
		if k = strings.TrimSpace(k); k != "" {
			out = append(out, k)
		}
	}
	return out
}

// eventPrinter writes rows as readable lines or raw NDJSON and remembers
// the newest seq of this device it printed.
type eventPrinter struct {
	w        io.Writer
	json     bool
	self     string // this device's id; other devices' rows are tagged
	lastSelf int64
	printed  int             // rows written so far
	seen     map[rowKey]bool // recent rows of a follow, skipped when they arrive live
}

// rowKey names one row of the log.
type rowKey struct {
	device string
	seq    int64
}

func (p *eventPrinter) event(ev store.Event) error {
	if k := (rowKey{ev.DeviceID, ev.Seq}); p.seen[k] {
		delete(p.seen, k)
		return nil
	}
	if ev.DeviceID == p.self && ev.Seq > p.lastSelf {
		p.lastSelf = ev.Seq
	}
	p.printed++
	if p.json {
		raw, err := json.Marshal(ev)
		if err != nil {
			return err
		}
		_, err = p.w.Write(append(raw, '\n'))
		return err
	}
	_, err := io.WriteString(p.w, formatEvent(ev, p.self, time.Local)+"\n")
	return err
}

// recent prints the newest limit rows, oldest first, and remembers them so
// an open stream does not print them again.
func (p *eventPrinter) recent(ctx context.Context, c *api.Client, kinds []string, limit int) error {
	if limit <= 0 {
		return nil
	}
	evs, err := c.Events(ctx, kinds, limit)
	if err != nil {
		return err
	}
	slices.Reverse(evs) // the route answers newest first
	if p.seen == nil {
		p.seen = map[rowKey]bool{}
	}
	for _, ev := range evs {
		if err := p.event(ev); err != nil {
			return errOutput
		}
		p.seen[rowKey{ev.DeviceID, ev.Seq}] = true
	}
	return nil
}

func (p *eventPrinter) resync() error {
	line := "-- some rows were dropped; resyncing --\n"
	if p.json {
		line = "{\"resync\":true}\n"
	}
	_, err := io.WriteString(p.w, line)
	return err
}

// summaryKeys come first in a row's summary, in this order; the rest of the
// payload's scalar fields follow sorted by name.
var summaryKeys = []string{"key", "name", "status", "reason", "kind", "repo", "path", "sha", "title"}

// maxSummary caps a row's summary in runes.
const maxSummary = 160

// formatEvent is one readable line: local time, kind, device (when not
// self) and the payload's scalar fields as k=v.
func formatEvent(ev store.Event, self string, loc *time.Location) string {
	var b strings.Builder
	b.WriteString(ev.HLC.Wall().In(loc).Format("01-02 15:04:05"))
	b.WriteByte(' ')
	fmt.Fprintf(&b, "%-16s", ev.Kind)
	if ev.DeviceID != self {
		b.WriteString(" [" + ev.DeviceID + "]")
	}
	if s := summarize(ev.Payload); s != "" {
		b.WriteByte(' ')
		b.WriteString(s)
	}
	if ev.Actor != "" && ev.Actor != "irori" {
		b.WriteString(" by=" + ev.Actor)
	}
	return strings.TrimRight(b.String(), " ")
}

// summarize renders a payload's scalar fields; nested objects and arrays
// are left out. A payload that is not an object is previewed as is.
func summarize(payload json.RawMessage) string {
	var m map[string]any
	if err := json.Unmarshal(payload, &m); err != nil {
		if len(payload) == 0 || string(payload) == "null" {
			return ""
		}
		return api.Preview(string(payload))
	}
	keys := make([]string, 0, len(m))
	for k := range m {
		if !slices.Contains(summaryKeys, k) {
			keys = append(keys, k)
		}
	}
	sort.Strings(keys)
	var parts []string
	for _, k := range append(slices.Clone(summaryKeys), keys...) {
		v, ok := m[k]
		if !ok {
			continue
		}
		var s string
		switch v := v.(type) {
		case string:
			if v == "" {
				continue
			}
			s = api.Preview(v)
			if strings.ContainsAny(s, " =\"") {
				s = strconv.Quote(s)
			}
		case float64:
			s = strconv.FormatFloat(v, 'f', -1, 64)
		case bool:
			s = strconv.FormatBool(v)
		default:
			continue // null, objects, arrays
		}
		parts = append(parts, k+"="+s)
	}
	out := strings.Join(parts, " ")
	if r := []rune(out); len(r) > maxSummary {
		out = string(r[:maxSummary-1]) + "…"
	}
	return out
}

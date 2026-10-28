package api

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
	"sort"
	"strconv"
	"strings"

	"github.com/StephenSHorton/irori/internal/store"
)

// NoReplay as EventsStream's since starts the stream live, replaying nothing.
const NoReplay int64 = -1

// maxEventLine caps one stream line; a longer one is dropped and reported
// as a resync, so the reader re-reads instead of silently missing it.
const maxEventLine = 8 << 20

// StreamItem is one item of EventsStream: an Event, a Resync signal (rows
// were dropped: re-read with Events), or a final Err just before the
// channel closes. A clean end (ctx cancelled, server shut down) has no Err.
// A Resync for a line over maxEventLine names the skipped row in
// SkippedDevice and SkippedSeq when its head was readable, so a reader
// resuming with since can move past it instead of being sent it again.
type StreamItem struct {
	Event         store.Event
	Resync        bool
	Err           error
	SkippedDevice string
	SkippedSeq    int64
}

// EventsStream opens GET /v1/events/stream (routes_events.go). kinds are
// exact kinds or prefixes ending in "."; since >= 0 replays this device's
// rows after that seq first, NoReplay starts live. An error opening the
// stream (a StatusError for a non-200) is returned directly; after that
// items arrive on the channel, which closes when the stream ends. Cancel
// ctx to stop it; the reader goroutine exits with it.
func (c *Client) EventsStream(ctx context.Context, kinds []string, since int64) (<-chan StreamItem, error) {
	sinceQ := ""
	if since >= 0 {
		sinceQ = strconv.FormatInt(since, 10)
	}
	return c.eventsStream(ctx, kinds, sinceQ)
}

// EventsStreamFrom is EventsStream with a vector since (v0.4 §6.8): each
// device it names is replayed after its seq; the rest arrive live. An
// empty vector replays nothing.
func (c *Client) EventsStreamFrom(ctx context.Context, kinds []string, since store.Vector) (<-chan StreamItem, error) {
	devs := make([]string, 0, len(since))
	for d := range since {
		devs = append(devs, d)
	}
	sort.Strings(devs)
	parts := make([]string, len(devs))
	for i, d := range devs {
		parts[i] = d + ":" + strconv.FormatInt(since[d], 10)
	}
	return c.eventsStream(ctx, kinds, strings.Join(parts, ","))
}

func (c *Client) eventsStream(ctx context.Context, kinds []string, since string) (<-chan StreamItem, error) {
	const path = "/v1/events/stream"
	q := url.Values{}
	if len(kinds) > 0 {
		q.Set("kinds", strings.Join(kinds, ","))
	}
	if since != "" {
		q.Set("since", since)
	}
	u := c.impl.base + path
	if len(q) > 0 {
		u += "?" + q.Encode()
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, u, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Bearer "+c.impl.lf.Token)
	req.Header.Set("Accept", "application/x-ndjson")
	hc := *c.impl.http
	hc.Timeout = 0 // a stream lives as long as ctx
	resp, err := hc.Do(req)
	if err != nil {
		return nil, fmt.Errorf("irori api %s: %w", path, err)
	}
	if resp.StatusCode != http.StatusOK {
		defer resp.Body.Close()
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 64<<10))
		var eb errorBody
		_ = json.Unmarshal(body, &eb)
		return nil, &StatusError{Path: path, Code: resp.StatusCode, Message: eb.Error}
	}
	ch := make(chan StreamItem)
	go func() {
		defer close(ch)
		defer resp.Body.Close()
		send := func(it StreamItem) bool {
			select {
			case ch <- it:
				return true
			case <-ctx.Done():
				return false
			}
		}
		r := bufio.NewReaderSize(resp.Body, 64<<10)
		for {
			line, skipped, err := readEventLine(r)
			if skipped != nil {
				it := StreamItem{Resync: true}
				it.SkippedDevice, it.SkippedSeq = lineHead(skipped)
				if !send(it) {
					return
				}
			}
			if len(line) > 0 {
				it, perr := parseStreamLine(line)
				if perr != nil {
					it = StreamItem{Err: fmt.Errorf("irori api %s: bad line: %w", path, perr)}
				}
				if !send(it) || perr != nil {
					return
				}
			}
			if err != nil {
				if !errors.Is(err, io.EOF) && ctx.Err() == nil {
					send(StreamItem{Err: fmt.Errorf("irori api %s: %w", path, err)})
				}
				return
			}
		}
	}()
	return ch, nil
}

// parseStreamLine reads one non-empty stream line.
func parseStreamLine(line []byte) (StreamItem, error) {
	var head struct {
		Resync bool  `json:"resync"`
		Seq    int64 `json:"seq"`
	}
	if err := json.Unmarshal(line, &head); err != nil {
		return StreamItem{}, err
	}
	if head.Resync {
		return StreamItem{Resync: true}, nil
	}
	var ev store.Event
	if err := json.Unmarshal(line, &ev); err != nil {
		return StreamItem{}, err
	}
	return StreamItem{Event: ev}, nil
}

// readEventLine returns the next line trimmed of spaces (a keepalive comes
// back empty). A line over maxEventLine is skipped whole, and its start (up
// to the reader's buffer size) comes back as skipped instead.
func readEventLine(r *bufio.Reader) (line, skipped []byte, err error) {
	var buf []byte
	for {
		chunk, err := r.ReadSlice('\n')
		if len(buf)+len(chunk) > maxEventLine {
			head := append(buf[:0:0], buf[:min(len(buf), r.Size())]...)
			if len(head) == 0 {
				head = append(head, chunk...)
			}
			for errors.Is(err, bufio.ErrBufferFull) {
				_, err = r.ReadSlice('\n')
			}
			return nil, head, err
		}
		buf = append(buf, chunk...)
		if errors.Is(err, bufio.ErrBufferFull) {
			continue
		}
		return bytes.TrimSpace(buf), nil, err
	}
}

// lineHead reads deviceId and seq from the start of a truncated event line.
// They come before the payload in a store.Event line; "", 0 when the start
// does not have them.
func lineHead(head []byte) (deviceID string, seq int64) {
	dec := json.NewDecoder(bytes.NewReader(head))
	dec.UseNumber()
	if t, err := dec.Token(); err != nil || t != json.Delim('{') {
		return "", 0
	}
	var gotDev, gotSeq bool
	for !gotDev || !gotSeq {
		k, err := dec.Token()
		key, ok := k.(string)
		if err != nil || !ok {
			return "", 0
		}
		v, err := dec.Token()
		if err != nil {
			return "", 0
		}
		switch key {
		case "deviceId":
			deviceID, gotDev = v.(string)
		case "seq":
			n, ok := v.(json.Number)
			if !ok {
				return "", 0
			}
			if seq, err = n.Int64(); err != nil {
				return "", 0
			}
			gotSeq = true
		}
		if _, ok := v.(json.Delim); ok {
			return "", 0 // a nested value before both fields: give up
		}
	}
	return deviceID, seq
}

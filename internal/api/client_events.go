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
type StreamItem struct {
	Event  store.Event
	Resync bool
	Err    error
}

// EventsStream opens GET /v1/events/stream (routes_events.go). kinds are
// exact kinds or prefixes ending in "."; since >= 0 replays this device's
// rows after that seq first, NoReplay starts live. An error opening the
// stream (a StatusError for a non-200) is returned directly; after that
// items arrive on the channel, which closes when the stream ends. Cancel
// ctx to stop it; the reader goroutine exits with it.
func (c *Client) EventsStream(ctx context.Context, kinds []string, since int64) (<-chan StreamItem, error) {
	const path = "/v1/events/stream"
	q := url.Values{}
	if len(kinds) > 0 {
		q.Set("kinds", strings.Join(kinds, ","))
	}
	if since >= 0 {
		q.Set("since", strconv.FormatInt(since, 10))
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
			line, oversized, err := readEventLine(r)
			if oversized && !send(StreamItem{Resync: true}) {
				return
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
// back empty). A line over maxEventLine is skipped whole and reported.
func readEventLine(r *bufio.Reader) (line []byte, oversized bool, err error) {
	var buf []byte
	for {
		chunk, err := r.ReadSlice('\n')
		if len(buf)+len(chunk) > maxEventLine {
			for errors.Is(err, bufio.ErrBufferFull) {
				_, err = r.ReadSlice('\n')
			}
			return nil, true, err
		}
		buf = append(buf, chunk...)
		if errors.Is(err, bufio.ErrBufferFull) {
			continue
		}
		return bytes.TrimSpace(buf), false, err
	}
}

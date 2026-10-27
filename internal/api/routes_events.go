package api

// Event stream route. CONTRACT for the api, CLI and monitor work packages:
// the path, query parameters and line shapes below are binding. It keeps
// v0.1's rules (bearer token, Host check, no Origin) and, like GET
// /v1/events, is a read open to any token holder.
//
//	GET /v1/events/stream?kinds=<k1,k2,...>&since=<seq>
//	    NDJSON, one object per line, for as long as the client stays.
//	    kinds is a comma list of exact kinds or prefixes ending in "."
//	    ("agent."); absent or empty = every kind.
//	    since is a seq of THIS device (GET /v1/device id): rows of this
//	    device with seq > since are replayed first, oldest first, then the
//	    stream goes live. since=0 replays all of this device's rows. Without
//	    since nothing is replayed. Rows replicated from other devices are
//	    never replayed (their seqs are not comparable); they arrive live.
//	    There is no gap and no duplicate between replay and live.
//	    Lines:
//	      store.Event           {"deviceId","seq","hlc","actor","kind","payload"}
//	      {"resync":true}       live rows were dropped (slow reader): re-read
//	                            with GET /v1/events or reconnect with since
//	      "" (a bare "\n")      keepalive every Keepalive (15 s) when idle
//	    400 when since is not a non-negative integer.

import (
	"context"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/store"
)

// resyncLine is the stream line for a dropped-rows signal.
type resyncLine struct {
	Resync bool `json:"resync"`
}

// parseKinds splits a comma list, dropping empty entries.
func parseKinds(s string) []string {
	var out []string
	for _, k := range strings.Split(s, ",") {
		if k = strings.TrimSpace(k); k != "" {
			out = append(out, k)
		}
	}
	return out
}

// matchKind reports whether kind passes kinds (exact, or a prefix ending
// in "."), the same rule as store.Filter.Kinds. No kinds passes everything.
func matchKind(kinds []string, kind string) bool {
	if len(kinds) == 0 {
		return true
	}
	for _, k := range kinds {
		if k == kind || (strings.HasSuffix(k, ".") && strings.HasPrefix(kind, k)) {
			return true
		}
	}
	return false
}

// eventsStream serves GET /v1/events/stream.
func (h *handler) eventsStream(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	kinds := parseKinds(q.Get("kinds"))
	since := int64(-1)
	if v, ok := q["since"]; ok {
		n, err := strconv.ParseInt(v[0], 10, 64)
		if err != nil || n < 0 {
			writeError(w, http.StatusBadRequest, "since must be a non-negative integer")
			return
		}
		since = n
	}

	ctx, cancel := context.WithCancel(r.Context())
	defer cancel()
	// Subscribe before the replay query, so a row appended in between is
	// in one or the other (or both, deduped below), never in neither.
	live := h.s.b.SubscribeEvents(ctx)
	self := h.s.b.Device().ID
	var replay []store.Event
	last := since
	if since >= 0 {
		for {
			evs, err := h.s.b.Events(ctx, store.Filter{Kinds: kinds, DeviceID: self, AfterSeq: last, Limit: MaxEventLimit})
			if err != nil {
				writeError(w, http.StatusInternalServerError, err.Error())
				return
			}
			replay = append(replay, evs...)
			if len(evs) > 0 {
				last = evs[len(evs)-1].Seq
			}
			if len(evs) < MaxEventLimit {
				break
			}
		}
	}

	// A stream outlives the server's read and write timeouts.
	rc := http.NewResponseController(w)
	_ = rc.SetReadDeadline(time.Time{})
	_ = rc.SetWriteDeadline(time.Time{})
	w.Header().Set("Content-Type", "application/x-ndjson")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(http.StatusOK)
	out := &lineWriter{w: w, rc: rc}
	if out.flush() != nil {
		return
	}
	for _, ev := range replay {
		if out.sendJSON(ev) != nil {
			return
		}
	}

	tick := time.NewTicker(Keepalive)
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-tick.C:
			if out.keepalive() != nil {
				return
			}
		case ev, ok := <-live:
			if !ok {
				return // the store closed
			}
			var err error
			switch {
			case ev.Seq == 0:
				err = out.sendJSON(resyncLine{Resync: true})
			case ev.DeviceID == self && ev.Seq <= last:
				continue // replayed already, or at or before since
			case !matchKind(kinds, ev.Kind):
				continue
			default:
				err = out.sendJSON(ev)
			}
			if err != nil {
				return
			}
			tick.Reset(Keepalive) // keepalives only fill silence
		}
	}
}

package api

// Event stream route. CONTRACT for the api, CLI and monitor work packages:
// the path, query parameters and line shapes below are binding. It keeps
// v0.1's rules (bearer token, Host check, no Origin) and, like GET
// /v1/events, is a read open to any token holder.
//
//	GET /v1/events/stream?kinds=<k1,k2,...>&since=<deviceId>:<seq>,...
//	    NDJSON, one object per line, for as long as the client stays.
//	    kinds is a comma list of exact kinds or prefixes ending in "."
//	    ("agent."); absent or empty = every kind.
//	    since is a vector (v0.4 §6.8): for each device named, its rows with
//	    seq > that seq are replayed first, oldest first, one device after
//	    another in device id order; then the stream goes live. A device the
//	    vector doesn't name is not replayed: its rows arrive live from now.
//	    <deviceId>:0 replays all of that device's rows. The old form, a bare
//	    seq (since=<seq>), is the vector of THIS device alone (GET
//	    /v1/device id): "my device only, the rest from now". Without since
//	    nothing is replayed. There is no gap and no duplicate between replay
//	    and live, per device.
//	    Lines:
//	      store.Event           {"deviceId","seq","hlc","actor","kind","payload"}
//	      {"resync":true}       live rows were dropped (slow reader): re-read
//	                            with GET /v1/events or reconnect with since
//	      "" (a bare "\n")      keepalive every Keepalive (15 s) when idle
//	    400 when since is neither a non-negative integer nor a vector of
//	    <deviceId>:<non-negative seq> entries.
//
// The person's view (decision D10(a)). GET /v1/events and the stream show
// rows whole only to a request carrying this run's X-Irori-Person secret
// (the GUI's). With the bearer token alone, or an agent's secret, rows read
// as the events tool shows them: message text and reasons, and turn
// prompts that are Irori fences, are "[left out]" (tools.Redact). A wrong
// or blank X-Irori-Person is 403.

import (
	"context"
	"fmt"
	"net/http"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/store"
	"github.com/StephenSHorton/irori/internal/tools"
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
	person, ok := h.rowsView(w, r)
	if !ok {
		return
	}
	self := h.s.b.Device().ID
	var since store.Vector // nil: replay nothing
	if v, ok := q["since"]; ok {
		var err error
		if since, err = parseSince(v[0], self); err != nil {
			writeError(w, http.StatusBadRequest, err.Error())
			return
		}
	}

	ctx, cancel := context.WithCancel(r.Context())
	defer cancel()
	// Subscribe before the replay query, so a row appended in between is
	// in one or the other (or both, deduped below), never in neither.
	live := h.s.b.SubscribeEvents(ctx)
	var replay []store.Event
	last := store.Vector{} // per device: the newest seq replayed (or since)
	devs := make([]string, 0, len(since))
	for d := range since {
		devs = append(devs, d)
	}
	sort.Strings(devs)
	for _, dev := range devs {
		last[dev] = since[dev]
		for {
			evs, err := h.s.b.Events(ctx, store.Filter{Kinds: kinds, DeviceID: dev, AfterSeq: last[dev], Limit: MaxEventLimit})
			if err != nil {
				writeError(w, http.StatusInternalServerError, err.Error())
				return
			}
			replay = append(replay, evs...)
			if len(evs) > 0 {
				last[dev] = evs[len(evs)-1].Seq
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
		if out.sendJSON(viewRow(ev, person)) != nil {
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
			case named(last, ev.DeviceID) && ev.Seq <= last[ev.DeviceID]:
				continue // replayed already, or at or before since
			case !matchKind(kinds, ev.Kind):
				continue
			default:
				err = out.sendJSON(viewRow(ev, person))
			}
			if err != nil {
				return
			}
			tick.Reset(Keepalive) // keepalives only fill silence
		}
	}
}

// parseSince reads since: a bare seq is self's alone (the v0.1 form), else
// a vector of <deviceId>:<seq> entries.
func parseSince(v, self string) (store.Vector, error) {
	if n, err := strconv.ParseInt(v, 10, 64); err == nil {
		if n < 0 {
			return nil, fmt.Errorf("since must be a non-negative integer or a vector of <deviceId>:<seq>")
		}
		return store.Vector{self: n}, nil
	}
	out := store.Vector{}
	for _, part := range strings.Split(v, ",") {
		part = strings.TrimSpace(part)
		if part == "" {
			continue
		}
		i := strings.LastIndexByte(part, ':')
		if i <= 0 {
			return nil, fmt.Errorf("since entry %q is not <deviceId>:<seq>", part)
		}
		n, err := strconv.ParseInt(part[i+1:], 10, 64)
		if err != nil || n < 0 {
			return nil, fmt.Errorf("since entry %q needs a non-negative seq", part)
		}
		dev := part[:i]
		if _, dup := out[dev]; dup {
			return nil, fmt.Errorf("since names %q twice", dev)
		}
		out[dev] = n
	}
	if len(out) == 0 {
		return nil, fmt.Errorf("since must be a non-negative integer or a vector of <deviceId>:<seq>")
	}
	return out, nil
}

func named(v store.Vector, dev string) bool {
	_, ok := v[dev]
	return ok
}

// rowsView reports whether r reads rows as the person (this run's
// X-Irori-Person secret, decision D10(a)); false with ok false means a 403
// was written for a wrong one.
func (h *handler) rowsView(w http.ResponseWriter, r *http.Request) (person, ok bool) {
	present, valid := h.s.asPerson(r)
	if present && !valid {
		writeError(w, http.StatusForbidden, "wrong "+HeaderPerson+" secret (only the Irori app holds it)")
		return false, false
	}
	return valid, true
}

// viewRow is ev as the reader may see it: whole for the person, else as
// the events tool shows it (tools.Redact).
func viewRow(ev store.Event, person bool) store.Event {
	if !person {
		ev.Payload = tools.Redact(ev.Kind, ev.Payload)
	}
	return ev
}

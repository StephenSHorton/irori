package api

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"log"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode"

	"github.com/StephenSHorton/irori/internal/attach"
	"github.com/StephenSHorton/irori/internal/inbox"
)

// Keepalive is how often an idle attach stream writes a bare "\n".
var Keepalive = 15 * time.Second

// countLimit bounds the deliveries scanned for inbox counts.
const countLimit = 1 << 20

// stream is one open attach stream (mod session or Grok member).
type stream struct {
	key    string // agent key
	path   inbox.Path
	sender inbox.Sender
	secret string        // attach secret; "" for member streams (they use the member secret)
	kick   chan struct{} // closed when a newer stream for key supersedes this one
	// appends: a mod at attach.AppendSince or later, which takes append
	// events (next-turn delivery). Older mods and Grok never get them.
	appends bool
}

// streams is the registry: one live stream per agent key. mu also
// serializes wake issuing, so a superseded stream can't take a wake after
// its successor has looked for outstanding ones.
type streams struct {
	mu      sync.Mutex
	byKey   map[string]*stream
	secrets map[string]*stream
}

func newStreams() *streams {
	return &streams{byKey: map[string]*stream{}, secrets: map[string]*stream{}}
}

func (r *streams) bySecret(secret string) *stream {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.secrets[secret]
}

// current reports whether st is still key's live stream. Callers hold mu.
func (r *streams) current(st *stream) bool { return r.byKey[st.key] == st }

func newSecret() (string, error) {
	var b [24]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", err
	}
	return hex.EncodeToString(b[:]), nil
}

// attach serves GET /v1/attach?session=<id>&mod=<v> or ?member=<secret>.
func (h *handler) attach(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	b := h.s.b
	ib := b.Inbox()
	st := &stream{kick: make(chan struct{})}
	hello := attach.Event{Type: attach.TypeHello, V: attach.Version, API: &attach.API{
		URL: "http://127.0.0.1:" + strconv.Itoa(h.port), Token: string(h.s.token),
	}}
	switch session, member := q.Get("session"), q.Get("member"); {
	case session != "" && member == "":
		if len(session) > 200 || strings.ContainsFunc(session, func(r rune) bool { return r == '/' || unicode.IsControl(r) }) {
			writeError(w, http.StatusBadRequest, "bad session id")
			return
		}
		key := b.Device().ID + "/" + session
		var name string
		found := false
		for _, c := range b.Agents() {
			if c.Key == key && c.Agent == "claude" && c.Status != "gone" {
				name, found = c.Name, true
				break
			}
		}
		if !found {
			writeError(w, http.StatusNotFound, "no live session "+strconv.Quote(session)+" on this device (yet)")
			return
		}
		secret, err := newSecret()
		if err != nil {
			writeError(w, http.StatusInternalServerError, err.Error())
			return
		}
		st.key, st.path, st.secret = key, inbox.PathMod, secret
		st.appends = attach.ModAtLeast(q.Get("mod"), attach.AppendSince)
		st.sender = inbox.Sender{Key: key, Name: name, Kind: "claude"}
		hello.Session, hello.AgentKey, hello.Name, hello.AttachSecret = session, key, name, secret
	case member != "" && session == "":
		m, ok := b.MemberBySecret(member)
		if !ok {
			writeError(w, http.StatusForbidden, "unknown member secret")
			return
		}
		st.key, st.path = m.Key, inbox.PathChannel
		st.sender = inbox.Sender{Key: m.Key, Name: m.Name, Kind: "grok"}
		hello.AgentKey, hello.Name = m.Key, m.Name
		if i := strings.LastIndexByte(m.Key, '/'); i >= 0 {
			hello.Session = m.Key[i+1:]
		}
	default:
		writeError(w, http.StatusBadRequest, "pass exactly one of session or member")
		return
	}

	// A stream outlives the server's read and write timeouts.
	rc := http.NewResponseController(w)
	_ = rc.SetReadDeadline(time.Time{})
	_ = rc.SetWriteDeadline(time.Time{})
	w.Header().Set("Content-Type", "application/x-ndjson")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(http.StatusOK)

	ctx, cancel := context.WithCancel(r.Context())
	defer cancel()
	changes := ib.Subscribe(ctx) // before anything can make a wake ready

	out := &lineWriter{w: w, rc: rc}
	reg := h.s.streams

	// Register (superseding an older stream) and mark reachable in one step.
	reg.mu.Lock()
	if old := reg.byKey[st.key]; old != nil {
		close(old.kick)
		delete(reg.secrets, old.secret)
	}
	reg.byKey[st.key] = st
	if st.secret != "" {
		reg.secrets[st.secret] = st
	}
	if st.path == inbox.PathChannel {
		b.MemberStream(st.key, true)
	}
	ib.SetReachable(st.key, st.path, true)
	reg.mu.Unlock()

	defer func() {
		reg.mu.Lock()
		if reg.current(st) {
			delete(reg.byKey, st.key)
			delete(reg.secrets, st.secret)
			ib.SetReachable(st.key, st.path, false)
		}
		// Every member stream opened one count, superseded or not, so every
		// one closes it; otherwise a supersede leaves the card listed for good.
		if st.path == inbox.PathChannel {
			b.MemberStream(st.key, false)
		}
		reg.mu.Unlock()
		select {
		case <-st.kick:
			_ = out.send(attach.Event{Type: attach.TypeSuperseded})
		default:
		}
	}()

	if out.send(hello) != nil {
		return
	}
	// A wake issued to an earlier stream but never confirmed delivered goes
	// out again with the same token (the mod dedupes on it); a confirmed one
	// is never re-sent.
	if !h.wake(ctx, st, out, true) || !h.sendAppend(ctx, st, out, true) {
		return
	}
	counts := h.counts(st.key)
	if out.send(counts) != nil {
		return
	}

	tick := time.NewTicker(Keepalive)
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-st.kick:
			return
		case <-tick.C:
			if out.keepalive() != nil {
				return
			}
			// Catch up on a dropped notification (slow subscriber).
			if !h.wake(ctx, st, out, false) || !h.sendAppend(ctx, st, out, false) {
				return
			}
		case ch, ok := <-changes:
			if !ok {
				return
			}
			if ch.To != "" && ch.To != st.key {
				continue
			}
			switch ch.Kind {
			case inbox.HeldNew:
				if ev, ok := h.heldEvent(ch.DeliveryID); ok && out.send(ev) != nil {
					return
				}
			case inbox.MovedAway:
				if d, ok := ib.Get(ch.DeliveryID); ok && out.send(attach.Event{Type: attach.TypeMoved, DeliveryID: d.ID, To: d.MovedTo}) != nil {
					return
				}
			}
			// Every change for this agent (AppendReady among them) looks for
			// a wake and an append; both are nil most of the time.
			if !h.wake(ctx, st, out, false) || !h.sendAppend(ctx, st, out, false) {
				return
			}
			if c := h.counts(st.key); c.Unread != counts.Unread || c.HeldN != counts.HeldN {
				counts = c
				if out.send(c) != nil {
					return
				}
			}
		}
	}
}

// wake issues and sends the next wake for st, if any. resend also re-sends
// an issued wake that was never confirmed delivered. false means the stream
// is done (write failed or st was superseded).
func (h *handler) wake(ctx context.Context, st *stream, out *lineWriter, resend bool) bool {
	ib := h.s.b.Inbox()
	reg := h.s.streams
	reg.mu.Lock()
	if !reg.current(st) {
		reg.mu.Unlock()
		return false
	}
	var w *inbox.Wake
	if resend {
		if ow, state := ib.OutstandingWake(st.key); ow != nil && state == inbox.Queued {
			w = ow
		}
	}
	if w == nil {
		var err error
		if w, err = ib.NextWake(ctx, st.key, st.path); err != nil {
			reg.mu.Unlock()
			if ctx.Err() == nil {
				log.Printf("attach %s: next wake: %v", st.key, err)
			}
			return true
		}
	}
	reg.mu.Unlock()
	if w == nil {
		return true
	}
	return out.send(h.deliverEvent(w)) == nil
}

// sendAppend issues and sends st's next append (next-turn delivery), if st
// takes them. resend sends an issued, unconfirmed one again first (same
// token; the mod dedupes on it). false means the stream is done.
func (h *handler) sendAppend(ctx context.Context, st *stream, out *lineWriter, resend bool) bool {
	if !st.appends {
		return true
	}
	ib := h.s.b.Inbox()
	reg := h.s.streams
	reg.mu.Lock()
	if !reg.current(st) {
		reg.mu.Unlock()
		return false
	}
	var w *inbox.Wake
	if resend {
		w = ib.OutstandingAppend(st.key)
	}
	if w == nil {
		var err error
		if w, err = ib.NextAppend(ctx, st.key); err != nil {
			reg.mu.Unlock()
			if ctx.Err() == nil {
				log.Printf("attach %s: next append: %v", st.key, err)
			}
			return true
		}
	}
	reg.mu.Unlock()
	if w == nil {
		return true
	}
	ev := h.deliverEvent(w)
	ev.Type, ev.Mode = attach.TypeAppend, string(inbox.ModeNextTurn)
	return out.send(ev) == nil
}

// deliverEvent is w on the wire: a deliver (Mode set only on a
// next_turn-only push), or the body of an append.
func (h *handler) deliverEvent(w *inbox.Wake) attach.Event {
	ev := attach.Event{
		Type: attach.TypeDeliver, WakeID: w.ID, Token: w.Token, Reminder: w.Reminder,
		DeliveryIDs: append([]string{}, w.DeliveryIDs...), Prompt: w.Prompt,
	}
	if w.Mode == inbox.ModeNextTurn {
		ev.Mode = string(inbox.ModeNextTurn)
	}
	if len(w.Overflow) > 0 {
		ev.Overflow = append([]string{}, w.Overflow...)
	}
	ib := h.s.b.Inbox()
	for _, id := range w.DeliveryIDs {
		if d, ok := ib.Get(id); ok {
			ev.MessageIDs = append(ev.MessageIDs, d.MessageID)
			if ev.From == "" {
				ev.From = d.Message.From.Name
			}
		}
	}
	return ev
}

func (h *handler) heldEvent(id string) (attach.Event, bool) {
	d, ok := h.s.b.Inbox().Get(id)
	if !ok || d.State != inbox.Held {
		return attach.Event{}, false
	}
	return attach.Event{
		Type: attach.TypeHeld, DeliveryID: d.ID, From: d.Message.From.Name,
		Why: string(d.Hold), Preview: Preview(d.Message.Text),
	}, true
}

// counts is the inbox event for key: open deliveries it can see, and those
// held from it.
func (h *handler) counts(key string) attach.Event {
	ib := h.s.b.Inbox()
	ev := attach.Event{Type: attach.TypeInbox, Unread: len(ib.For(key, false, countLimit))}
	for _, d := range ib.List(countLimit) {
		if d.To == key && d.State == inbox.Held {
			ev.HeldN++
		}
	}
	return ev
}

// Preview is text's first 120 runes on one line.
func Preview(text string) string {
	text = strings.Join(strings.FieldsFunc(text, func(r rune) bool {
		return unicode.IsSpace(r) || unicode.IsControl(r)
	}), " ")
	if r := []rune(text); len(r) > 120 {
		text = string(r[:120])
	}
	return text
}

// lineWriter writes NDJSON lines and flushes each one.
type lineWriter struct {
	mu sync.Mutex
	w  http.ResponseWriter
	rc *http.ResponseController
}

func (l *lineWriter) send(ev attach.Event) error { return l.sendJSON(ev) }

// sendJSON writes v as one line.
func (l *lineWriter) sendJSON(v any) error {
	raw, err := json.Marshal(v)
	if err != nil {
		return err
	}
	return l.write(append(raw, '\n'))
}

func (l *lineWriter) keepalive() error { return l.write([]byte{'\n'}) }

// flush sends the headers (and anything buffered) now.
func (l *lineWriter) flush() error {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.rc.Flush()
}

func (l *lineWriter) write(p []byte) error {
	l.mu.Lock()
	defer l.mu.Unlock()
	if _, err := l.w.Write(p); err != nil {
		return err
	}
	return l.rc.Flush()
}

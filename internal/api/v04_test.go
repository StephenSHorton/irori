package api

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/search"
	"github.com/StephenSHorton/irori/internal/store"
)

// remoteRows appends n rows on another device's log and inserts them into
// st, as the mesh would.
func remoteRows(t *testing.T, st *store.Store, dev string, n int) []store.Event {
	t.Helper()
	ctx := context.Background()
	other, err := store.Open(ctx, filepath.Join(t.TempDir(), dev+".db"), dev, hlc.New(time.Now))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { other.Close() })
	var out []store.Event
	for i := range n {
		ev, err := other.Append(ctx, "irori", "agent.status", map[string]int{"i": i})
		if err != nil {
			t.Fatal(err)
		}
		if err := st.Insert(ctx, ev); err != nil {
			t.Fatal(err)
		}
		out = append(out, ev)
	}
	return out
}

// The stream's since is a vector (v0.4 §6.8): each named device replays
// after its seq, an unnamed one arrives live only, and a bare seq still
// means this device alone.
func TestEventsStreamVectorSince(t *testing.T) {
	st, c := startStore(t)
	appendKinds(t, st, "agent.seen", "agent.status") // dev1: 1, 2
	remoteRows(t, st, "dev2", 3)                     // dev2: 1, 2, 3
	remoteRows(t, st, "dev3", 2)                     // dev3: never named
	type at struct {
		dev string
		seq int64
	}
	read := func(ch <-chan StreamItem, n int) []at {
		var got []at
		for range n {
			it := next(t, ch)
			got = append(got, at{it.Event.DeviceID, it.Event.Seq})
		}
		return got
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	ch, err := c.EventsStreamFrom(ctx, nil, store.Vector{"dev1": 1, "dev2": 1})
	if err != nil {
		t.Fatal(err)
	}
	if got := read(ch, 3); got[0] != (at{"dev1", 2}) || got[1] != (at{"dev2", 2}) || got[2] != (at{"dev2", 3}) {
		t.Fatalf("replay = %v", got)
	}
	// Live: another device's new row arrives, nothing replayed comes twice.
	ev, err := st.Append(ctx, "irori", "agent.status", map[string]int{})
	if err != nil {
		t.Fatal(err)
	}
	if got := read(ch, 1); got[0] != (at{"dev1", ev.Seq}) {
		t.Fatalf("live = %v", got)
	}

	// The old form: this device only, the rest from now.
	ch2, err := c.EventsStream(ctx, nil, 1)
	if err != nil {
		t.Fatal(err)
	}
	if got := read(ch2, 2); got[0] != (at{"dev1", 2}) || got[1] != (at{"dev1", 3}) {
		t.Fatalf("bare since = %v", got)
	}

	for _, bad := range []string{"dev1:x", "dev1:-1", ":3", "dev1", "dev1:1,dev1:2", ","} {
		resp := do(t, personPort(t, c), http.MethodGet, "/v1/events/stream?since="+bad, bearer(testToken))
		if resp.StatusCode != http.StatusBadRequest {
			t.Errorf("since=%q: %d, want 400", bad, resp.StatusCode)
		}
	}
}

func personPort(t *testing.T, c *Client) int {
	t.Helper()
	return c.impl.lf.Port
}

// Without the GUI's secret, event rows read as the events tool shows them
// (D10(a)): no message text, no fenced prompt.
func TestEventsLeaveOutMessagesWithoutThePerson(t *testing.T) {
	st, c := startStore(t)
	ctx := context.Background()
	port := personPort(t, c)
	if _, err := st.Append(ctx, "dev1/s1", "message.sent", map[string]any{"id": "msg_1", "text": "secret mango", "reason": "ripe kiwi"}); err != nil {
		t.Fatal(err)
	}
	if _, err := st.Append(ctx, "irori", "capture.turn", map[string]any{"key": "dev1/s1", "prompt": `<irori-message from="a">papaya</irori-message>`}); err != nil {
		t.Fatal(err)
	}
	get := func(path string, hdr map[string]string) (int, string) {
		resp := do(t, port, http.MethodGet, path, func(r *http.Request) {
			bearer(testToken)(r)
			for k, v := range hdr {
				r.Header.Set(k, v)
			}
		})
		b, _ := io.ReadAll(resp.Body)
		return resp.StatusCode, string(b)
	}
	code, body := get("/v1/events", nil)
	if code != 200 || strings.Contains(body, "mango") || strings.Contains(body, "kiwi") || strings.Contains(body, "papaya") || !strings.Contains(body, "[left out]") {
		t.Fatalf("token alone: %d %s", code, body)
	}
	code, body = get("/v1/events", map[string]string{HeaderPerson: personOf(port)})
	if code != 200 || !strings.Contains(body, "mango") || !strings.Contains(body, "papaya") {
		t.Fatalf("the person: %d %s", code, body)
	}
	if code, _ := get("/v1/events", map[string]string{HeaderPerson: "guess"}); code != http.StatusForbidden {
		t.Fatalf("wrong person secret: %d", code)
	}

	// The stream follows the same rule.
	sctx, cancel := context.WithCancel(ctx)
	defer cancel()
	ch, err := c.EventsStream(sctx, []string{"message."}, 0)
	if err != nil {
		t.Fatal(err)
	}
	var p map[string]any
	_ = json.Unmarshal(next(t, ch).Event.Payload, &p)
	if p["text"] != "[left out]" || p["reason"] != "[left out]" {
		t.Fatalf("stream payload = %v", p)
	}
	resp := do(t, port, http.MethodGet, "/v1/events/stream?kinds=message.&since=0", func(r *http.Request) {
		bearer(testToken)(r)
		r.Header.Set(HeaderPerson, personOf(port))
	})
	line := make([]byte, 4096)
	n, _ := resp.Body.Read(line)
	if !strings.Contains(string(line[:n]), "mango") {
		t.Fatalf("the person's stream: %s", line[:n])
	}
}

// The bearer token alone is an unidentified agent on the board routes
// (D10(a)): it reads no message and no overview.
func TestBoardTokenAloneIsAnAgent(t *testing.T) {
	f := newBoardFixture(t)
	ctx := context.Background()
	anon := Identity{}
	res, err := f.client.Search(ctx, anon, search.Query{Text: "mango"})
	if err != nil {
		t.Fatal(err)
	}
	for _, h := range res.Hits {
		if strings.HasPrefix(h.Ref, "msg:") {
			t.Fatalf("the token alone found a message: %+v", h)
		}
	}
	if _, err := f.client.Show(ctx, anon, "msg:"+f.msgToGrok, search.ShowOptions{}); statusOf(err) != http.StatusNotFound {
		t.Fatalf("show a message with the token alone: %v", err)
	}
	if _, err := f.client.Overview(ctx, anon, grokKey); statusOf(err) != http.StatusForbidden {
		t.Fatalf("overview with the token alone: %v", err)
	}
	if _, err := f.client.Board(ctx, anon, coord.BoardFilter{}); err != nil {
		t.Fatalf("the board with the token alone: %v", err)
	}
	// The person's secret still reads everything.
	if _, err := f.client.Show(ctx, f.asPerson(), "msg:"+f.msgToGrok, search.ShowOptions{}); err != nil {
		t.Fatalf("the person's show: %v", err)
	}
}

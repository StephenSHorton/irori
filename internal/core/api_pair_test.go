package core

import (
	"bytes"
	"context"
	"errors"
	"net/http"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/api"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/pair"
)

// The joining side of pairing is on the running Core's API (§3, §6.8), so
// `irori pair join` works while the app runs. These drive it through the
// API on the joining Core, with the person's side on the adding Core.

func pairClient(t *testing.T, h *hcore) *api.Client {
	t.Helper()
	c, err := api.Dial(context.Background(), h.data)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func apiState(t *testing.T, c *api.Client, want string) func() bool {
	return func() bool {
		st, err := c.PairStatus(context.Background())
		if err != nil {
			t.Fatal(err)
		}
		return st.State == want
	}
}

// A join through the API pairs the running Core: the words come back in
// the status, a wrong key counts a miss, the right one finishes it.
func TestJoinThroughTheAPIWhileTheCoreRuns(t *testing.T) {
	ctx := context.Background()
	net := mesh.NewFakeNet()
	pc := newHCore(t, net, "pc", "")
	mac := newHCore(t, net, "mac", "")
	c := pairClient(t, mac)

	add, err := pc.c.Pairing().Add(identity.RoleDesktop)
	if err != nil {
		t.Fatal(err)
	}
	st, err := c.PairJoin(ctx, strings.ToLower(add.Code), "")
	if err != nil {
		t.Fatal(err)
	}
	if st.Mode != pair.ModeJoin || st.State != pair.StateWaiting {
		t.Fatalf("join status = %+v", st)
	}
	wait(t, "the mac shows the words", apiState(t, c, pair.StateWords))
	st, _ = c.PairStatus(ctx)
	if len(st.Words) != 5 || st.Peer != pc.c.Device().Name {
		t.Fatalf("words status = %+v", st)
	}
	wait(t, "the pc asks for them", func() bool { return pc.c.Pairing().Status().State == pair.StateConfirm })
	if err := pc.c.Pairing().Confirm(ctx, strings.Join(st.Words, " ")); err != nil {
		t.Fatal(err)
	}
	if _, err := c.PairKey(ctx, "0000 0000 0000 0000"); !api.IsWrongKey(err) {
		t.Fatalf("a wrong key: %v", err)
	}
	if st, _ := c.PairStatus(ctx); st.State != pair.StateWords || st.Misses != 1 {
		t.Fatalf("after one wrong key = %+v", st)
	}
	if _, err := c.PairKey(ctx, add.Fingerprint); err != nil {
		t.Fatal(err)
	}
	wait(t, "paired", func() bool {
		return pc.c.Pairing().Status().State == pair.StateDone && mac.c.Pairing().Status().State == pair.StateDone
	})
	if mac.c.Person().ID != pc.c.Person().ID {
		t.Fatal("the mac didn't take the pc's person")
	}
	// A paired device can't be pulled into another person over the API.
	if _, err := c.PairJoin(ctx, pair.NewCode(), ""); err == nil || !strings.Contains(err.Error(), "409") {
		t.Fatalf("a paired device joined again: %v", err)
	}
}

// The second wrong key closes the window on both sides.
func TestJoinThroughTheAPIWrongKeyTwice(t *testing.T) {
	ctx := context.Background()
	net := mesh.NewFakeNet()
	pc := newHCore(t, net, "pc", "")
	mac := newHCore(t, net, "mac", "")
	c := pairClient(t, mac)

	add, err := pc.c.Pairing().Add(identity.RoleDesktop)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := c.PairJoin(ctx, add.Code, ""); err != nil {
		t.Fatal(err)
	}
	wait(t, "the mac shows the words", apiState(t, c, pair.StateWords))
	for range pair.MaxMisses {
		if _, err := c.PairKey(ctx, "1111 2222 3333 4444"); !api.IsWrongKey(err) {
			t.Fatalf("a wrong key: %v", err)
		}
	}
	st, _ := c.PairStatus(ctx)
	if st.State != pair.StateFailed || st.Error != pair.ErrTextNotThatKey {
		t.Fatalf("after two wrong keys = %+v", st)
	}
	wait(t, "the pc's window closes", func() bool { return !pc.c.Pairing().Status().Open() })
	_, err = c.PairKey(ctx, add.Fingerprint)
	var se *api.StatusError
	if !errors.As(err, &se) || se.Code != http.StatusConflict || se.Message != "no pairing is waiting for a key" {
		t.Fatalf("a key after the window closed: %v", err)
	}
	if pc.c.Pairing().Status().State == pair.StateDone || mac.c.Person().ID == pc.c.Person().ID {
		t.Fatal("paired after two wrong keys")
	}
}

// A code nobody answers expires; cancel closes an open join.
func TestJoinThroughTheAPIExpiresAndCancels(t *testing.T) {
	ctx := context.Background()
	mac := &hcore{t: t, name: "mac", data: t.TempDir(), claude: t.TempDir(), net: mesh.NewFakeNet(), clk: &clock{}}
	mac.edit = func(o *Options) { o.PairWindow = 300 * time.Millisecond }
	mac.start()
	t.Cleanup(mac.stop)
	c := pairClient(t, mac)

	if _, err := c.PairJoin(ctx, "not a code", ""); err == nil || !strings.Contains(err.Error(), "400") {
		t.Fatalf("a bad code: %v", err)
	}
	if _, err := c.PairJoin(ctx, pair.NewCode(), "no port here"); err == nil || !strings.Contains(err.Error(), "400") {
		t.Fatalf("a bad server address: %v", err)
	}
	if _, err := c.PairJoin(ctx, pair.NewCode(), ""); err != nil {
		t.Fatal(err)
	}
	wait(t, "the code expires", apiState(t, c, pair.StateExpired))

	mac.stop()
	mac.edit = nil // the full window: only cancel closes it now
	mac.start()
	c = pairClient(t, mac)
	if _, err := c.PairJoin(ctx, pair.NewCode(), "203.0.113.7:24817"); err != nil {
		t.Fatal(err)
	}
	// Here, not under the short window: each join derives its room key
	// (scrypt) before it checks for an open one, which under -race
	// outlasts 300ms, so the first would expire before this was refused.
	if _, err := c.PairJoin(ctx, pair.NewCode(), ""); err == nil || !strings.Contains(err.Error(), "409") {
		t.Fatalf("a second join while one is open: %v", err)
	}
	if st, _ := c.PairStatus(ctx); st.Via != "203.0.113.7:24817" || !st.Open() {
		t.Fatalf("join from away = %+v", st)
	}
	st, err := c.PairCancel(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if st.State != pair.StateCanceled {
		t.Fatalf("after cancel = %+v", st)
	}
}

// Nothing on the API opens a pairing window, types the words or removes a
// device; while the person's Add window is open the API shows neither its
// code nor its key, and its cancel leaves the window open.
func TestThePairingAPIHasNoAddSide(t *testing.T) {
	ctx := context.Background()
	net := mesh.NewFakeNet()
	pc := newHCore(t, net, "pc", "")
	mac := newHCore(t, net, "mac", "")
	pairCores(t, pc, mac, identity.RoleDesktop)
	lf, err := discovery.Read(pc.data)
	if err != nil {
		t.Fatal(err)
	}
	call := func(method, path string) int {
		req, err := http.NewRequestWithContext(ctx, method, "http://127.0.0.1:"+strconv.Itoa(lf.Port)+path,
			bytes.NewReader([]byte(`{"role":"desktop","words":"a b c d e","deviceId":"`+mac.id()+`"}`)))
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Authorization", "Bearer "+lf.Token)
		req.Header.Set("Content-Type", "application/json")
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		resp.Body.Close()
		return resp.StatusCode
	}
	for _, path := range []string{"/v1/pair/add", "/v1/pair/open", "/v1/pair/code", "/v1/pair/words", "/v1/pair/confirm",
		"/v1/pair/revoke", "/v1/pair/remove", "/v1/pairing/add", "/v1/devices/add", "/v1/devices/" + mac.id(), "/v1/devices/revoke"} {
		for _, m := range []string{http.MethodGet, http.MethodPost, http.MethodPut, http.MethodDelete} {
			if code := call(m, path); code < 400 {
				t.Fatalf("%s %s = %d", m, path, code)
			}
		}
	}
	if call(http.MethodGet, "/v1/pair/add") != http.StatusNotFound {
		t.Fatal("GET /v1/pair/add is not a 404")
	}
	if call(http.MethodGet, "/v1/pair/join") != http.StatusMethodNotAllowed {
		t.Fatal("GET /v1/pair/join is allowed")
	}
	if st := pc.c.Pairing().Status(); st.Mode == pair.ModeAdd && st.Open() {
		t.Fatalf("the API opened a window: %+v", st)
	}
	if rows := pc.c.Mesh().Devices(false); len(rows) != 2 {
		t.Fatalf("devices = %d", len(rows))
	}
	for _, r := range pc.c.Mesh().Devices(false) {
		if r.Revoked {
			t.Fatalf("the API removed %s", r.DeviceID)
		}
	}

	add, err := pc.c.Pairing().Add(identity.RoleNode)
	if err != nil {
		t.Fatal(err)
	}
	c := pairClient(t, pc)
	st, err := c.PairStatus(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if st.Mode != pair.ModeAdd || st.State != pair.StateWaiting || st.Code != "" || st.Fingerprint != "" || st.ServerAddr != "" || st.Role != "" {
		t.Fatalf("the API shows the Add window = %+v", st)
	}
	if _, err := c.PairKey(ctx, add.Fingerprint); err == nil {
		t.Fatal("a key typed into an Add window")
	}
	if _, err := c.PairCancel(ctx); err != nil {
		t.Fatal(err)
	}
	if got := pc.c.Pairing().Status(); got.State != pair.StateWaiting || got.Code != add.Code {
		t.Fatalf("the API's cancel closed the Add window: %s", got.State)
	}
	if _, err := c.PairJoin(ctx, pair.NewCode(), ""); err == nil || !strings.Contains(err.Error(), "409") {
		t.Fatalf("joined while an Add window is open: %v", err)
	}
}

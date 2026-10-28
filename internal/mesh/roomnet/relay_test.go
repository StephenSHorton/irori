package roomnet

import (
	"fmt"
	"strings"
	"testing"
	"time"

	room "github.com/StephenSHorton/go-room"

	"github.com/StephenSHorton/irori/internal/mesh"
)

// A credential URL's query (often an API key) never reaches a status line
// or the log, and a relay set while the transport runs shows at once.
func TestRelayErrorsDropTheURLsQuery(t *testing.T) {
	var logged []string
	tr := New(Options{Self: "me", Relay: mesh.Relay{CredentialURL: "http://127.0.0.1:1/cred?api_key=hunter2"},
		Logf: func(f string, a ...any) { logged = append(logged, fmt.Sprintf(f, a...)) }})
	defer tr.Close()
	_ = tr.ice(nil) // starts the credential loop; port 1 refuses
	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) && tr.RelayStatus().State != room.RelayError {
		time.Sleep(20 * time.Millisecond)
	}
	st := tr.RelayStatus()
	if st.State != room.RelayError {
		t.Fatalf("relay status %+v, want error", st)
	}
	tr.Close()
	for _, s := range append(logged, st.Detail) {
		if strings.Contains(s, "hunter2") {
			t.Fatalf("the credential URL's key leaked: %q", s)
		}
	}
	tr2 := New(Options{Self: "me"})
	if tr2.RelayStatus().State != room.RelayNone {
		t.Fatal("no relay set is not Direct only")
	}
	tr2.SetRelay(mesh.Relay{CredentialURL: "https://relay.example/cred"})
	if tr2.RelayStatus().State != room.RelayPending {
		t.Fatal("a relay set later doesn't show")
	}
}

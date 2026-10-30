//go:build darwin

// The Go v0.5.0 updater taking the Rust v0.6.0 release on the Mac
// (architecture.md 8 M8, tools/rehearse/rehearse.sh; the PC's twin is
// update_rehearse_test.go). Injected into I:internal/update with
// go test -overlay, so the client is Go's own UpdateService and macUpdate
// code: its releases/latest read, asset matchers, SHA256SUMS check,
// download, stage (ditto unpack, bundle and signature checks, the
// .Irori.app.new copy) and InstallOnExit starting the swap helper. Two
// things point elsewhere: apiBase, at a fake GitHub on 127.0.0.1
// (httptest) that serves the Rust .app.zip, and the live bundle, a scratch
// copy of Irori.app instead of the one this process runs from.
//
// This test process stands in for the running Go GUI: the swap helper waits
// for it to exit, so the script polls after it returns. InstallOnExit
// (relaunch 0), because a relaunch goes through `open`, which would start
// the swapped app without the scratch environment.
//
// Env (all set by rehearse.sh):
//
//	IRORI_REHEARSE_APPZIP   the Rust irori-0.6.0-macos-arm64.app.zip
//	IRORI_REHEARSE_LIVEAPP  the scratch Irori.app the update replaces
//	IRORI_REHEARSE_PIDFILE  where this process writes its pid
//	IRORI_HOME              a scratch dir: New() downloads into
//	                        <IRORI_HOME>/updates
package update

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

const rehearseMacAsset = "irori-0.6.0-macos-arm64.app.zip"

func TestRehearseGoUpdatesToRustMac(t *testing.T) {
	zip := os.Getenv("IRORI_REHEARSE_APPZIP")
	if zip == "" {
		t.Skip("IRORI_REHEARSE_APPZIP not set (tools/rehearse/rehearse.sh runs this)")
	}
	home := os.Getenv("IRORI_HOME")
	if home == "" || !strings.Contains(strings.ToLower(home), "rehearse") {
		t.Fatalf("IRORI_HOME must be a rehearsal scratch dir, got %q", home)
	}
	live := os.Getenv("IRORI_REHEARSE_LIVEAPP")
	if !strings.Contains(strings.ToLower(live), "rehearse") || strings.HasPrefix(live, "/Applications/") ||
		filepath.Base(live) != "Irori.app" {
		t.Fatalf("IRORI_REHEARSE_LIVEAPP must be a scratch Irori.app, got %q", live)
	}
	data, err := os.ReadFile(zip)
	if err != nil {
		t.Fatal(err)
	}
	sum := sha256.Sum256(data)
	hexSum := hex.EncodeToString(sum[:])

	var srv *httptest.Server
	mux := http.NewServeMux()
	mux.HandleFunc("/repos/"+Repo+"/releases/latest", func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewEncoder(w).Encode(ghRelease{
			TagName:     "v0.6.0",
			Body:        "The first Rust build (rehearsal).",
			PublishedAt: "2026-10-29T00:00:00Z",
			Assets: []ghAsset{
				{Name: "irori-0.6.0-windows-amd64-installer.exe", URL: srv.URL + "/dl/irori-0.6.0-windows-amd64-installer.exe"},
				{Name: rehearseMacAsset, URL: srv.URL + "/dl/" + rehearseMacAsset},
				{Name: "SHA256SUMS", URL: srv.URL + "/dl/SHA256SUMS"},
			},
		})
	})
	mux.HandleFunc("/dl/"+rehearseMacAsset, func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write(data)
	})
	mux.HandleFunc("/dl/SHA256SUMS", func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprintf(w, "%s  %s\n%s  %s\n", strings.Repeat("0", 64), "irori-0.6.0-windows-amd64-installer.exe", hexSum, rehearseMacAsset)
	})
	srv = httptest.NewServer(mux) // 127.0.0.1
	defer srv.Close()
	if !strings.HasPrefix(srv.URL, "http://127.0.0.1:") {
		t.Fatalf("the fake feed is not on 127.0.0.1: %s", srv.URL)
	}

	s := New(Repo, "0.5.0")
	s.apiBase = srv.URL
	s.plat = macPlatform(s, func() (string, error) { return live, nil }, os.Getpid)
	wantDir := filepath.Join(home, "updates")
	if s.dir != wantDir {
		t.Fatalf("download dir %q, want %q under the scratch IRORI_HOME", s.dir, wantDir)
	}
	quit := 0
	s.SetHooks(nil, func() { quit++ })

	st, err := s.CheckAndDownload()
	if err != nil {
		t.Fatalf("CheckAndDownload: %v", err)
	}
	if st.Phase != PhaseReady || st.Version != "0.6.0" {
		t.Fatalf("status %+v, want ready for 0.6.0", st)
	}
	next, _ := sidePaths(live)
	if st.InstallerPath != next {
		t.Fatalf("staged at %q, want %q next to the scratch bundle", st.InstallerPath, next)
	}
	if v, err := plistString(filepath.Join(next, "Contents", "Info.plist"), "CFBundleShortVersionString"); err != nil || v != "0.6.0" {
		t.Fatalf("staged bundle version %q (%v), want 0.6.0", v, err)
	}
	got, err := sha256File(filepath.Join(home, "updates", rehearseMacAsset))
	if err != nil || got != hexSum {
		t.Fatalf("downloaded zip hash %s (%v), want %s", got, err, hexSum)
	}
	t.Logf("go updater: v0.6.0 verified against SHA256SUMS and staged as %s", filepath.Base(next))

	if err := os.WriteFile(os.Getenv("IRORI_REHEARSE_PIDFILE"), []byte(strconv.Itoa(os.Getpid())), 0o644); err != nil {
		t.Fatal(err)
	}
	s.InstallOnExit()
	if quit != 0 {
		t.Fatalf("quit called %d times on exit, want 0", quit)
	}
	if p := s.GetStatus().Phase; p != PhaseInstalling {
		t.Fatalf("phase %q after the launch, want installing", p)
	}
	t.Log("go updater: swap helper started (relaunch 0), exiting")
}

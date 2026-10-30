//go:build windows

// The Go v0.5.0 updater taking the Rust v0.6.0 release (architecture.md
// 8 M8, tools/rehearse/rehearse.ps1). Injected into I:internal/update with
// go test -overlay, so the client is Go's own UpdateService code: its
// releases/latest read, asset matchers, SHA256SUMS check, download, and
// RestartToUpdate / InstallOnExit launching the installer with
// /S [/NORELAUNCH] /WAITPID=<this pid>. Only apiBase points elsewhere: at a
// fake GitHub on 127.0.0.1 (httptest) that serves the Rust installer.
//
// This test process stands in for the running Go GUI: the installer waits
// for it to exit (/WAITPID), so the script polls after it returns.
//
// Env (all set by rehearse.ps1):
//
//	IRORI_REHEARSE_INSTALLER  the Rust installer, built with IRORI_SCRATCH
//	                          and IRORI_SCRATCH_INSTDIR (Go passes no /D=)
//	IRORI_REHEARSE_MODE       restart (Restart now) or exit (quit with the
//	                          update ready: /NORELAUNCH)
//	IRORI_REHEARSE_PIDFILE    where this process writes its pid
//	IRORI_HOME                a scratch dir: New() downloads into
//	                          <IRORI_HOME>/updates, and the relaunched Rust
//	                          build opens it as its data dir
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

const rehearseAsset = "irori-0.6.0-windows-amd64-installer.exe"

func TestRehearseGoUpdatesToRust(t *testing.T) {
	inst := os.Getenv("IRORI_REHEARSE_INSTALLER")
	if inst == "" {
		t.Skip("IRORI_REHEARSE_INSTALLER not set (tools/rehearse/rehearse.ps1 runs this)")
	}
	home := os.Getenv("IRORI_HOME")
	if home == "" || !strings.Contains(strings.ToLower(home), "rehearse") {
		t.Fatalf("IRORI_HOME must be a rehearsal scratch dir, got %q", home)
	}
	mode := os.Getenv("IRORI_REHEARSE_MODE")
	if mode != "restart" && mode != "exit" {
		t.Fatalf("IRORI_REHEARSE_MODE must be restart or exit, got %q", mode)
	}
	data, err := os.ReadFile(inst)
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
				{Name: "irori-0.6.0-windows-amd64.zip", URL: srv.URL + "/dl/irori-0.6.0-windows-amd64.zip"},
				{Name: rehearseAsset, URL: srv.URL + "/dl/" + rehearseAsset},
				{Name: "SHA256SUMS", URL: srv.URL + "/dl/SHA256SUMS"},
			},
		})
	})
	mux.HandleFunc("/dl/"+rehearseAsset, func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write(data)
	})
	mux.HandleFunc("/dl/SHA256SUMS", func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprintf(w, "%s  %s\n%s  %s\n", strings.Repeat("0", 64), "irori-0.6.0-windows-amd64.zip", hexSum, rehearseAsset)
	})
	srv = httptest.NewServer(mux) // 127.0.0.1
	defer srv.Close()
	if !strings.HasPrefix(srv.URL, "http://127.0.0.1:") {
		t.Fatalf("the fake feed is not on 127.0.0.1: %s", srv.URL)
	}

	s := New(Repo, "0.5.0")
	s.apiBase = srv.URL
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
	got, err := sha256File(st.InstallerPath)
	if err != nil || got != hexSum {
		t.Fatalf("downloaded installer hash %s (%v), want %s", got, err, hexSum)
	}
	t.Logf("go updater: v0.6.0 ready (%s, verified against SHA256SUMS)", filepath.Base(st.InstallerPath))

	if err := os.WriteFile(os.Getenv("IRORI_REHEARSE_PIDFILE"), []byte(strconv.Itoa(os.Getpid())), 0o644); err != nil {
		t.Fatal(err)
	}
	switch mode {
	case "restart":
		if err := s.RestartToUpdate(); err != nil {
			t.Fatalf("RestartToUpdate: %v", err)
		}
		if quit != 1 {
			t.Fatalf("quit called %d times, want 1", quit)
		}
		t.Log("go updater: installer started with /S /WAITPID, quitting")
	case "exit":
		s.InstallOnExit()
		if quit != 0 {
			t.Fatalf("quit called %d times on exit, want 0", quit)
		}
		t.Log("go updater: installer started with /S /NORELAUNCH /WAITPID, exiting")
	}
	if p := s.GetStatus().Phase; p != PhaseInstalling {
		t.Fatalf("phase %q after the launch, want installing", p)
	}
}

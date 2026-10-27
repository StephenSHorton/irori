package update

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"
)

func TestIsNewer(t *testing.T) {
	cases := []struct {
		tag, current string
		want         bool
	}{
		{"v1.2.3", "1.2.2", true},
		{"v1.2.3", "1.2.3", false},
		{"v1.2.3", "1.3.0", false},
		{"v2.0.0", "1.9.9", true},
		{"v1.0.0", "1.0.0-rc.1", true}, // a release beats its own prerelease
		{"v1.0.0-rc.1", "1.0.0", false},
		{"v1.2.3+build.7", "1.2.3", false}, // build metadata is ignored
		{"v1.2.3", "dev", false},           // dev builds never nag
		{"v1.2.3", "", false},
		{"1.2.3", "1.2.0", true}, // tolerates a missing leading v
	}
	for _, c := range cases {
		if got := isNewer(c.tag, c.current); got != c.want {
			t.Errorf("isNewer(%q, %q) = %v, want %v", c.tag, c.current, got, c.want)
		}
	}
}

func TestCmpSemver(t *testing.T) {
	if cmpSemver("1.10.0", "1.9.0") <= 0 {
		t.Error("1.10.0 should sort above 1.9.0 (numeric, not lexical)")
	}
	if cmpSemver("1.2.3", "1.2.3") != 0 {
		t.Error("equal versions should compare 0")
	}
}

// fakeLauncher records every installer launch instead of running one.
type fakeLauncher struct {
	calls [][]string
	err   error
}

func (f *fakeLauncher) launch(path string, args ...string) error {
	f.calls = append(f.calls, append([]string{path}, args...))
	return f.err
}

const testInstaller = `C:\Temp\irori-update\irori-1.2.3-windows-amd64-installer.exe`

// readyService returns a service parked in "ready" with a fake launcher and no
// app (Quit and Emit are nil-guarded).
func readyService(t *testing.T) (*UpdateService, *fakeLauncher) {
	t.Helper()
	s := New("x/y", "1.0.0")
	f := &fakeLauncher{}
	s.launch = f.launch
	s.status = Status{Phase: PhaseReady, Version: "1.2.3", InstallerPath: testInstaller}
	return s, f
}

func TestRestartToUpdateLaunchesSilent(t *testing.T) {
	s, f := readyService(t)
	var seen []string
	s.SetOnChange(func(st Status) { seen = append(seen, st.Phase) })

	if err := s.RestartToUpdate(); err != nil {
		t.Fatalf("RestartToUpdate: %v", err)
	}
	want := [][]string{{testInstaller, "/S"}}
	if !reflect.DeepEqual(f.calls, want) {
		t.Fatalf("launches = %v, want %v", f.calls, want)
	}
	if got := s.GetStatus().Phase; got != PhaseInstalling {
		t.Fatalf("phase = %q, want installing", got)
	}
	if !reflect.DeepEqual(seen, []string{PhaseInstalling}) {
		t.Fatalf("onChange phases = %v, want [installing]", seen)
	}
}

func TestRestartToUpdateEmitsAndQuits(t *testing.T) {
	s, _ := readyService(t)
	var events []string
	quits := 0
	s.SetHooks(func(name string, data any) {
		events = append(events, name+":"+data.(Status).Phase)
	}, func() { quits++ })

	if err := s.RestartToUpdate(); err != nil {
		t.Fatal(err)
	}
	if want := []string{EventUpdateStatus + ":" + PhaseInstalling}; !reflect.DeepEqual(events, want) {
		t.Fatalf("events = %v, want %v", events, want)
	}
	if quits != 1 {
		t.Fatalf("quit called %d times, want 1", quits)
	}
}

func TestRestartToUpdateRequiresReady(t *testing.T) {
	s, f := readyService(t)
	s.status = Status{Phase: PhaseDownloading, Version: "1.2.3"}
	if err := s.RestartToUpdate(); err == nil {
		t.Fatal("expected an error when no update is ready")
	}
	if len(f.calls) != 0 {
		t.Fatalf("launched %v; want nothing", f.calls)
	}
}

func TestRestartToUpdateLaunchFailureRevertsToReady(t *testing.T) {
	s, f := readyService(t)
	f.err = errors.New("boom")
	if err := s.RestartToUpdate(); err == nil {
		t.Fatal("expected launch error")
	}
	if got := s.GetStatus().Phase; got != PhaseReady {
		t.Fatalf("phase = %q, want ready (retryable)", got)
	}
}

func TestInstallOnExitLaunchesNoRelaunch(t *testing.T) {
	s, f := readyService(t)
	s.InstallOnExit()
	want := [][]string{{testInstaller, "/S", "/NORELAUNCH"}}
	if !reflect.DeepEqual(f.calls, want) {
		t.Fatalf("launches = %v, want %v", f.calls, want)
	}
	if got := s.GetStatus().Phase; got != PhaseInstalling {
		t.Fatalf("phase = %q, want installing", got)
	}
}

func TestInstallOnExitOnlyWhenReady(t *testing.T) {
	for _, phase := range []string{PhaseIdle, PhaseChecking, PhaseDownloading, PhaseInstalling, PhaseError} {
		s, f := readyService(t)
		s.status.Phase = phase
		s.InstallOnExit()
		if len(f.calls) != 0 {
			t.Errorf("phase %q: launched %v; want nothing", phase, f.calls)
		}
	}
}

func TestNeitherLaunchesTwice(t *testing.T) {
	// The real sequence: Restart now -> app.Quit -> OnShutdown -> InstallOnExit.
	s, f := readyService(t)
	if err := s.RestartToUpdate(); err != nil {
		t.Fatal(err)
	}
	s.InstallOnExit()
	if err := s.RestartToUpdate(); err == nil {
		t.Error("second RestartToUpdate should refuse while installing")
	}
	if want := [][]string{{testInstaller, "/S"}}; !reflect.DeepEqual(f.calls, want) {
		t.Fatalf("launches = %v, want exactly %v", f.calls, want)
	}

	// Quit without restarting: InstallOnExit twice launches once.
	s, f = readyService(t)
	s.InstallOnExit()
	s.InstallOnExit()
	if len(f.calls) != 1 {
		t.Fatalf("launches = %v, want exactly one", f.calls)
	}
}

func TestCheckAndDownloadReturnsReadyStatusWithoutRerun(t *testing.T) {
	s, f := readyService(t)
	st, err := s.CheckAndDownload()
	if err != nil {
		t.Fatal(err)
	}
	if st.Phase != PhaseReady || st.Version != "1.2.3" {
		t.Fatalf("status = %+v, want the ready status back", st)
	}
	if len(f.calls) != 0 {
		t.Fatalf("launched %v; want nothing", f.calls)
	}
}

// fakeGitHub serves a latest release with one installer and SHA256SUMS on a
// loopback server. sums overrides the SHA256SUMS body ("" = correct hash,
// "-" = omit the asset).
type fakeGitHub struct {
	tag, sums string
	payload   []byte
	hits      atomic.Int32
}

const testAsset = "irori-1.2.3-windows-amd64-installer.exe"

func (g *fakeGitHub) serve(t *testing.T) *httptest.Server {
	t.Helper()
	var srv *httptest.Server
	mux := http.NewServeMux()
	mux.HandleFunc("/repos/x/y/releases/latest", func(w http.ResponseWriter, r *http.Request) {
		g.hits.Add(1)
		assets := []ghAsset{{Name: testAsset, URL: srv.URL + "/dl/" + testAsset}}
		if g.sums != "-" {
			assets = append(assets, ghAsset{Name: "SHA256SUMS", URL: srv.URL + "/dl/SHA256SUMS"})
		}
		_ = json.NewEncoder(w).Encode(ghRelease{TagName: g.tag, Body: "notes", Assets: assets})
	})
	mux.HandleFunc("/dl/"+testAsset, func(w http.ResponseWriter, r *http.Request) { _, _ = w.Write(g.payload) })
	mux.HandleFunc("/dl/SHA256SUMS", func(w http.ResponseWriter, r *http.Request) {
		body := g.sums
		if body == "" {
			h := sha256.Sum256(g.payload)
			body = fmt.Sprintf("%s  irori-1.2.3-windows-amd64.zip\n%s  %s\n",
				strings.Repeat("0", 64), hex.EncodeToString(h[:]), testAsset)
		}
		_, _ = w.Write([]byte(body))
	})
	srv = httptest.NewServer(mux)
	t.Cleanup(srv.Close)
	return srv
}

func testService(t *testing.T, srv *httptest.Server, current string) *UpdateService {
	t.Helper()
	s := New("x/y", current)
	s.apiBase = srv.URL
	s.dir = t.TempDir()
	return s
}

func TestFindUpdatePicksInstallerAndChecksum(t *testing.T) {
	g := &fakeGitHub{tag: "v1.2.3", payload: []byte("installer bytes")}
	s := testService(t, g.serve(t), "1.0.0")
	info, err := s.findUpdate()
	if err != nil {
		t.Fatal(err)
	}
	h := sha256.Sum256(g.payload)
	if info == nil || info.Version != "1.2.3" || info.AssetName != testAsset || info.SHA256 != hex.EncodeToString(h[:]) {
		t.Fatalf("info = %+v", info)
	}
}

func TestFindUpdateCurrentOrDev(t *testing.T) {
	g := &fakeGitHub{tag: "v1.2.3", payload: []byte("x")}
	srv := g.serve(t)
	if info, err := testService(t, srv, "1.2.3").findUpdate(); err != nil || info != nil {
		t.Fatalf("same version: info=%+v err=%v, want nil, nil", info, err)
	}
	before := g.hits.Load()
	if info, err := testService(t, srv, "dev").findUpdate(); err != nil || info != nil {
		t.Fatalf("dev: info=%+v err=%v, want nil, nil", info, err)
	}
	if g.hits.Load() != before {
		t.Fatal("a dev build must not query GitHub at all")
	}
}

func TestFindUpdateNoReleases(t *testing.T) {
	srv := httptest.NewServer(http.NotFoundHandler())
	t.Cleanup(srv.Close)
	if info, err := testService(t, srv, "1.0.0").findUpdate(); err != nil || info != nil {
		t.Fatalf("info=%+v err=%v, want nil, nil", info, err)
	}
}

func TestFindUpdateRequiresChecksum(t *testing.T) {
	for _, sums := range []string{"-", "deadbeef  some-other-file.exe\n"} {
		g := &fakeGitHub{tag: "v1.2.3", payload: []byte("x"), sums: sums}
		if _, err := testService(t, g.serve(t), "1.0.0").findUpdate(); err == nil {
			t.Errorf("sums %q: expected an error for an unverifiable installer", sums)
		}
	}
}

func TestDownloadVerified(t *testing.T) {
	g := &fakeGitHub{tag: "v1.2.3", payload: []byte("installer bytes")}
	s := testService(t, g.serve(t), "1.0.0")
	info, err := s.findUpdate()
	if err != nil {
		t.Fatal(err)
	}
	path, err := s.downloadVerified(*info)
	if err != nil {
		t.Fatal(err)
	}
	if got, _ := os.ReadFile(path); string(got) != "installer bytes" {
		t.Fatalf("downloaded %q", got)
	}
	if filepath.Dir(path) != s.dir {
		t.Fatalf("installer at %s, want inside %s", path, s.dir)
	}

	info.SHA256 = strings.Repeat("0", 64)
	if _, err := s.downloadVerified(*info); err == nil {
		t.Fatal("expected a checksum mismatch")
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("mismatched installer left on disk (stat err %v)", err)
	}
}

func TestCheckAndDownloadPlatform(t *testing.T) {
	g := &fakeGitHub{tag: "v1.2.3", payload: []byte("installer bytes")}
	s := testService(t, g.serve(t), "1.0.0")
	var seen []string
	s.SetOnChange(func(st Status) { seen = append(seen, st.Phase) })
	st, err := s.CheckAndDownload()

	if !supported {
		if err == nil || st.Phase != PhaseError || !strings.Contains(st.Error, "not supported") {
			t.Fatalf("status %+v err %v, want an unsupported error", st, err)
		}
		if g.hits.Load() != 0 {
			t.Fatal("an unsupported platform must not query GitHub")
		}
		return
	}
	if err != nil {
		t.Fatal(err)
	}
	if st.Phase != PhaseReady || st.Version != "1.2.3" || st.InstallerPath == "" {
		t.Fatalf("status = %+v, want ready", st)
	}
	if want := []string{PhaseChecking, PhaseDownloading, PhaseReady}; !reflect.DeepEqual(seen, want) {
		t.Fatalf("phases = %v, want %v", seen, want)
	}
}

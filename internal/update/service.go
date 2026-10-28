// Package update is Irori's in-app updater: a background check that downloads
// a newer release, announces it, and lets the user pick when to restart.
//
// It reads the latest GitHub Release of Repo, compares its tag against the
// running version (main.version, injected with -ldflags -X), downloads this
// platform's asset, verifies it against the release's SHA256SUMS, and parks
// in the "ready" phase. From there the user restarts right away
// (RestartToUpdate: install, quit, and Irori comes back) or keeps working,
// and the install runs when Irori quits (InstallOnExit: no relaunch).
// Nothing installs or quits behind the user's back, and "dev" builds never
// offer updates. What differs by OS is a platform (nsis.go, mac_darwin.go).
//
// Windows runs the release's NSIS installer. The GUI is not the only process
// running irori.exe: every agent session runs `irori mcp` and `irori serve
// --headless` may be running too, so the exe image can stay locked after the
// GUI quits. The installer therefore waits for the GUI process itself
// (/WAITPID=<pid>), and if the exe is still locked it renames it aside to
// irori.exe.old (Windows allows renaming a running exe) and writes the new
// one in its place. Those running processes keep the old image until they
// restart; new sessions get the new exe. CleanupOldExes, called from
// AutoUpdate, deletes the renamed copies once nothing holds them.
//
// macOS replaces the whole Irori.app: the release's .app.zip is unpacked,
// its signature, bundle id and version checked, and copied beside the
// running bundle as .Irori.app.new while Irori still runs, so "ready" has
// already proven the folder is writable. A detached helper waits for the GUI
// to exit, swaps the two bundles with renames (running `irori mcp`
// processes keep the image they already opened), and reopens Irori unless
// it was quitting. Moving the signed bundle whole keeps its signature
// intact, which is why suzuri's re-sign and Info.plist heal steps (needed
// there because it swaps only the Mach-O) are not ported.
//
// Wails v3's own pkg/updater is not used: it swaps the exe in place, which
// leaves an NSIS install's registry entries, uninstaller and shortcuts stale.
//
// Linux cannot update itself yet: the check reports that updates are not
// supported (check_other.go), so the install paths are never reached there.
//
// JS binding name: UpdateService.*
package update

import (
	"fmt"
	"log"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/StephenSHorton/irori/internal/paths"
)

// Repo is the GitHub repository whose releases Irori updates from.
const Repo = "StephenSHorton/irori"

// EventUpdateStatus is emitted (payload: Status) on every phase transition so
// an open window can show the download, the "ready" banner or the restart.
const EventUpdateStatus = "update:status"

// Update phases, in the order of a normal run. "error" can follow any of the
// first three; "idle" also means "up to date" after a check that found nothing.
const (
	PhaseIdle        = "idle"
	PhaseChecking    = "checking"
	PhaseDownloading = "downloading"
	PhaseReady       = "ready"
	PhaseInstalling  = "installing"
	PhaseError       = "error"
)

// Status is the updater's state as the frontend and tray see it.
type Status struct {
	Phase   string `json:"phase"`   // idle|checking|downloading|ready|installing|error
	Version string `json:"version"` // target version once known, e.g. "0.2.0"
	Notes   string `json:"notes"`   // release body (markdown)
	// InstallerPath is the verified installer on disk once Phase is ready. The
	// JS side has no use for it, so it is not serialised.
	InstallerPath string `json:"-"`
	Error         string `json:"error"` // set while Phase is error
}

// UpdateInfo describes an available update.
type UpdateInfo struct {
	Version     string `json:"version"`     // tag without the leading v
	Notes       string `json:"notes"`       // release body (markdown)
	AssetURL    string `json:"assetUrl"`    // browser_download_url of the installer
	AssetName   string `json:"assetName"`   // installer filename
	SHA256      string `json:"sha256"`      // expected hash from SHA256SUMS
	PublishedAt string `json:"publishedAt"` // ISO-8601
}

// UpdateService is the Wails-bound updater API.
type UpdateService struct {
	repo    string // "owner/repo"
	current string // running version ("dev" in non-release builds)
	client  *http.Client

	// emit and quit reach the running app (see SetHooks); nil until then.
	// The package stays free of Wails so it vets and tests on any OS.
	emit func(name string, data any)
	quit func()

	// apiBase is the GitHub API root and dir the download folder; both are
	// swapped in tests.
	apiBase string
	dir     string

	mu       sync.Mutex
	status   Status
	onChange func(Status)
	inFlight atomic.Bool // one check+download at a time

	// launch starts the installer (or the macOS swap helper) detached.
	// Swapped for a fake in tests.
	launch func(path string, args ...string) error

	// plat is what updating means on this OS (newPlatform).
	plat platform
}

// platform is the part of an update that differs by OS.
type platform struct {
	// assetKind names the release asset in errors ("windows installer").
	assetKind string
	// isAsset picks this platform's asset from a release.
	isAsset func(name string) bool
	// precheck, if set, runs before the network check: a reason this copy
	// of Irori can't update itself (not running from an .app bundle).
	precheck func() error
	// stage, if set, turns the verified download into what install takes
	// (macOS unpacks and checks the .app). Its result is InstallerPath.
	stage func(downloaded string, info UpdateInfo) (string, error)
	// install starts installing staged and returns; the caller then quits.
	// relaunch false means the user is quitting and Irori stays closed.
	install func(staged string, relaunch bool) error
	// cleanup removes what an earlier update left behind; AutoUpdate runs it.
	cleanup func()
}

// New returns an UpdateService for the given GitHub repo ("owner/repo",
// normally Repo) and running version.
func New(repo, currentVersion string) *UpdateService {
	dir := filepath.Join(os.TempDir(), "irori-update")
	if data, err := paths.DataDir(); err == nil {
		dir = filepath.Join(data, "updates")
	}
	s := &UpdateService{
		repo:    repo,
		current: currentVersion,
		client:  &http.Client{Timeout: 30 * time.Second},
		apiBase: "https://api.github.com",
		dir:     dir,
		status:  Status{Phase: PhaseIdle},
	}
	s.launch = func(path string, args ...string) error {
		// Started detached so it outlives our exit.
		cmd := exec.Command(path, args...)
		detach(cmd)
		return cmd.Start()
	}
	s.plat = newPlatform(s)
	return s
}

// CheckForUpdate returns the available update, or nil if Irori is up to date
// (or a "dev" build, which never reports updates).
func (s *UpdateService) CheckForUpdate() (*UpdateInfo, error) {
	if !supported {
		return nil, errUnsupported
	}
	if isDev(s.current) {
		return nil, nil
	}
	if s.plat.precheck != nil {
		if err := s.plat.precheck(); err != nil {
			return nil, err
		}
	}
	return s.findUpdate()
}

// SetHooks connects the running app: emit sends a Wails event (main passes
// app.Event.Emit) and quit quits it before the installer runs (app.Quit).
// The event payload type is registered in package main
// (application.RegisterEvent[update.Status](update.EventUpdateStatus)).
//
//wails:ignore
func (s *UpdateService) SetHooks(emit func(name string, data any), quit func()) {
	s.mu.Lock()
	s.emit, s.quit = emit, quit
	s.mu.Unlock()
}

// SetOnChange registers a callback fired after every status transition, e.g.
// to rebuild the tray menu when an update becomes ready.
//
//wails:ignore
func (s *UpdateService) SetOnChange(fn func(Status)) {
	s.mu.Lock()
	s.onChange = fn
	s.mu.Unlock()
}

// GetVersion returns the running version string.
func (s *UpdateService) GetVersion() string { return s.current }

// Supported reports whether this platform can update itself (Windows and macOS).
func (s *UpdateService) Supported() bool { return supported }

// GetStatus returns the current updater status.
func (s *UpdateService) GetStatus() Status {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.status
}

// setStatus mutates the status under the lock, then notifies listeners
// outside it: the event emitter and the tray rebuild never run under mu.
func (s *UpdateService) setStatus(mut func(st *Status)) Status {
	s.mu.Lock()
	mut(&s.status)
	st := s.status
	cb, emit := s.onChange, s.emit
	s.mu.Unlock()
	if emit != nil {
		emit(EventUpdateStatus, st)
	}
	if cb != nil {
		cb(st)
	}
	return st
}

// AutoUpdate is the startup check: if a newer release exists it downloads and
// verifies it in the background and parks in "ready". It never installs or
// quits on its own. Errors are logged and swallowed, since an offline or
// failed check must never block startup. Call it in a goroutine.
func (s *UpdateService) AutoUpdate() {
	if !supported {
		log.Printf("irori: %s", errUnsupported)
		return
	}
	if s.plat.cleanup != nil {
		s.plat.cleanup()
	}
	if _, err := s.CheckAndDownload(); err != nil {
		log.Printf("irori: auto-update failed: %v", err)
	}
}

// CheckAndDownload is the manual "Check for updates" path: check, download and
// verify synchronously, and return the final status. One run is in flight at a
// time; a concurrent caller (the startup check racing a click) gets the current
// status back. Once an update is ready or installing there is nothing to do,
// so that status is returned as is.
func (s *UpdateService) CheckAndDownload() (Status, error) {
	if !s.inFlight.CompareAndSwap(false, true) {
		return s.GetStatus(), nil
	}
	defer s.inFlight.Store(false)

	if st := s.GetStatus(); st.Phase == PhaseReady || st.Phase == PhaseInstalling {
		return st, nil
	}

	s.setStatus(func(st *Status) { *st = Status{Phase: PhaseChecking} })
	info, err := s.CheckForUpdate()
	if err != nil {
		return s.fail(err), err
	}
	if info == nil {
		// Up to date, or a dev build.
		return s.setStatus(func(st *Status) { *st = Status{Phase: PhaseIdle} }), nil
	}

	log.Printf("irori: downloading update v%s", info.Version)
	s.setStatus(func(st *Status) {
		*st = Status{Phase: PhaseDownloading, Version: info.Version, Notes: info.Notes}
	})
	path, err := s.downloadVerified(*info)
	if err != nil {
		return s.fail(err), err
	}
	if s.plat.stage != nil {
		if path, err = s.plat.stage(path, *info); err != nil {
			return s.fail(err), err
		}
	}

	st := s.setStatus(func(st *Status) {
		st.Phase, st.InstallerPath, st.Error = PhaseReady, path, ""
	})
	log.Printf("irori: update v%s ready at %s", info.Version, path)
	return st, nil
}

// fail records an error status and logs it, keeping the version if known.
func (s *UpdateService) fail(err error) Status {
	log.Printf("irori: update: %v", err)
	return s.setStatus(func(st *Status) {
		st.Phase, st.Error, st.InstallerPath = PhaseError, err.Error(), ""
	})
}

// RestartToUpdate starts the install and quits Irori. On Windows the silent
// installer waits for this process to exit (/WAITPID), installs (renaming the
// old exe aside if `irori mcp`/`serve` still hold it), and relaunches Irori;
// if it cannot install at all it relaunches the old exe, so "Restart now"
// never leaves Irori closed (build/windows/nsis/project.nsi). On macOS the
// swap helper does the same with the .app bundle (mac_darwin.go).
func (s *UpdateService) RestartToUpdate() error {
	s.mu.Lock()
	if s.status.Phase != PhaseReady {
		phase := s.status.Phase
		s.mu.Unlock()
		return fmt.Errorf("no update ready to install (phase %q)", phase)
	}
	path, quit := s.status.InstallerPath, s.quit
	// Flip to installing BEFORE quitting: app.Quit runs OnShutdown, whose
	// InstallOnExit must see that an install is already under way.
	s.status.Phase = PhaseInstalling
	s.mu.Unlock()

	if err := s.plat.install(path, true); err != nil {
		s.setStatus(func(st *Status) { st.Phase = PhaseReady })
		return fmt.Errorf("launch installer: %w", err)
	}
	log.Printf("irori: restarting to update (installer %s)", path)
	s.setStatus(func(*Status) {}) // emit the installing transition
	if quit != nil {
		quit()
	}
	return nil
}

// InstallOnExit belongs in the app's OnShutdown. If an update is ready and
// nothing is installing, it starts the install without relaunch and returns
// at once: the process exits, the installer (Windows: /NORELAUNCH) or swap
// helper (macOS) waits for it, installs, and does not bring Irori back (the
// user chose to quit).
//
//wails:ignore
func (s *UpdateService) InstallOnExit() {
	s.mu.Lock()
	if s.status.Phase != PhaseReady {
		s.mu.Unlock()
		return
	}
	path := s.status.InstallerPath
	s.status.Phase = PhaseInstalling
	s.mu.Unlock()

	if err := s.plat.install(path, false); err != nil {
		log.Printf("irori: install on exit: launch installer: %v", err)
		return
	}
	log.Printf("irori: installing update on exit (installer %s)", path)
}

// isNewer reports whether tag (vX.Y.Z) is newer than current. A "dev" or empty
// current never reports an update, so `wails3 dev` never nags.
func isNewer(tag, current string) bool {
	if isDev(current) {
		return false
	}
	return cmpSemver(strings.TrimPrefix(tag, "v"), strings.TrimPrefix(current, "v")) > 0
}

func isDev(current string) bool { return current == "" || current == "dev" }

func cmpSemver(a, b string) int {
	na, prea := parseVer(a)
	nb, preb := parseVer(b)
	for i := range 3 {
		if na[i] != nb[i] {
			if na[i] > nb[i] {
				return 1
			}
			return -1
		}
	}
	// Same X.Y.Z: a final release outranks a prerelease.
	switch {
	case prea == "" && preb == "":
		return 0
	case prea == "":
		return 1
	case preb == "":
		return -1
	default:
		return strings.Compare(prea, preb)
	}
}

// parseVer returns the numeric [major,minor,patch] and the prerelease string
// (the part after '-'; build metadata after '+' is dropped).
func parseVer(v string) ([3]int, string) {
	v, _, _ = strings.Cut(v, "+")
	num, pre, _ := strings.Cut(v, "-")
	var out [3]int
	for i, p := range strings.SplitN(num, ".", 3) {
		n, _ := strconv.Atoi(strings.TrimSpace(p))
		out[i] = n
	}
	return out, pre
}

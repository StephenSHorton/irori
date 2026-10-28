//go:build darwin

package update

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"syscall"

	"golang.org/x/sys/unix"
)

// supported: macOS updates by swapping in the release's Irori.app.
const supported = true

var errUnsupported error

func newPlatform(s *UpdateService) platform { return macPlatform(s, runningApp, os.Getpid) }

// detach puts the swap helper in its own session, so nothing that ends the
// GUI's session (a terminal closing on `irori` started from a shell) takes
// the helper with it.
func detach(cmd *exec.Cmd) { cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true} }

// macUpdate holds the macOS platform's hooks. app finds the live bundle and
// pid is the process the swap waits for; both are swapped in tests.
type macUpdate struct {
	s   *UpdateService
	app func() (string, error)
	pid func() int
}

func macPlatform(s *UpdateService, app func() (string, error), pid func() int) platform {
	m := &macUpdate{s: s, app: app, pid: pid}
	return platform{
		assetKind: "macOS " + runtime.GOARCH + " app",
		isAsset:   func(name string) bool { return isMacAppAsset(name, runtime.GOARCH) },
		precheck:  func() error { _, err := m.app(); return err },
		stage:     m.stage,
		install:   m.install,
		cleanup:   m.cleanup,
	}
}

// runningApp is the Irori.app this process runs from, when it can be
// updated in place: a real bundle, not App Translocation's read-only copy,
// in a folder this user can write.
func runningApp() (string, error) {
	exe, err := os.Executable()
	if err != nil {
		return "", err
	}
	if exe, err = filepath.EvalSymlinks(exe); err != nil {
		return "", err
	}
	app := appBundleRoot(exe)
	switch {
	case app == "":
		return "", errors.New("Irori is not running from Irori.app, so it can't update itself; install the new version from the release page")
	case translocated(app):
		return "", errors.New("macOS is running Irori from a temporary read-only copy: move Irori.app into Applications and open it from there to get updates")
	}
	if err := unix.Access(filepath.Dir(app), unix.W_OK); err != nil {
		return "", fmt.Errorf("can't update Irori: %s is not writable for this user (%v); install the new version from the release page", filepath.Dir(app), err)
	}
	return app, nil
}

// stage unpacks the verified .app.zip, checks the bundle inside, and copies
// it next to the live bundle as .Irori.app.new. It returns that path.
func (m *macUpdate) stage(zipPath string, info UpdateInfo) (string, error) {
	live, err := m.app()
	if err != nil {
		return "", err
	}
	unpack := filepath.Join(m.s.dir, "unpacked")
	_ = os.RemoveAll(unpack)
	defer func() { _ = os.RemoveAll(unpack) }()
	// ditto, not archive/zip: it keeps the bundle's symlinks, modes and
	// extended attributes, which the code signature covers.
	if out, err := exec.Command("/usr/bin/ditto", "-x", "-k", zipPath, unpack).CombinedOutput(); err != nil {
		return "", fmt.Errorf("unpack %s: %v: %s", filepath.Base(zipPath), err, strings.TrimSpace(string(out)))
	}
	staged, err := onlyApp(unpack)
	if err != nil {
		return "", err
	}
	if err := checkBundle(staged, info.Version); err != nil {
		return "", err
	}
	if err := checkSignature(live, staged); err != nil {
		return "", err
	}
	// Go's HTTP client sets no quarantine, but a copy that has one would
	// meet Gatekeeper on relaunch.
	_ = exec.Command("/usr/bin/xattr", "-dr", "com.apple.quarantine", staged).Run()

	next, _ := sidePaths(live)
	_ = os.RemoveAll(next)
	if out, err := exec.Command("/usr/bin/ditto", staged, next).CombinedOutput(); err != nil {
		_ = os.RemoveAll(next)
		return "", fmt.Errorf("stage the update next to %s: %v: %s", live, err, strings.TrimSpace(string(out)))
	}
	return next, nil
}

// onlyApp returns the single .app at the top of dir.
func onlyApp(dir string) (string, error) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return "", err
	}
	var apps []string
	for _, e := range entries {
		if e.IsDir() && strings.HasSuffix(e.Name(), ".app") {
			apps = append(apps, filepath.Join(dir, e.Name()))
		}
	}
	if len(apps) != 1 {
		return "", fmt.Errorf("update zip holds %d .app bundles, want 1", len(apps))
	}
	return apps[0], nil
}

// checkBundle refuses a bundle that is not Irori at the version the release
// claims, or has no executable.
func checkBundle(app, version string) error {
	plist := filepath.Join(app, "Contents", "Info.plist")
	id, err := plistString(plist, "CFBundleIdentifier")
	if err != nil {
		return err
	}
	if id != macBundleID {
		return fmt.Errorf("update bundle id is %q, want %q", id, macBundleID)
	}
	v, err := plistString(plist, "CFBundleShortVersionString")
	if err != nil {
		return err
	}
	if want := numericVersion(version); v != want {
		return fmt.Errorf("update bundle is version %q, want %q", v, want)
	}
	exe, err := plistString(plist, "CFBundleExecutable")
	if err != nil {
		return err
	}
	st, err := os.Stat(filepath.Join(app, "Contents", "MacOS", exe))
	if err != nil || !st.Mode().IsRegular() || st.Mode()&0o111 == 0 {
		return fmt.Errorf("update bundle has no executable Contents/MacOS/%s", exe)
	}
	return nil
}

func plistString(plist, key string) (string, error) {
	out, err := exec.Command("/usr/bin/plutil", "-extract", key, "raw", "-o", "-", plist).Output()
	if err != nil {
		return "", fmt.Errorf("read %s from %s: %v", key, plist, err)
	}
	return strings.TrimSpace(string(out)), nil
}

// checkSignature refuses a staged bundle whose signature does not verify,
// and one that would replace a Developer ID signed Irori with a weaker
// signature: Gatekeeper would then call the installed app damaged (the
// lesson of suzuri's 0.9.87 update).
func checkSignature(live, staged string) error {
	if out, err := exec.Command("/usr/bin/codesign", "--verify", "--deep", "--strict", staged).CombinedOutput(); err != nil {
		return fmt.Errorf("update signature does not verify: %v: %s", err, strings.TrimSpace(string(out)))
	}
	if developerID(live) && !developerID(staged) {
		return errors.New("this Irori is signed with Developer ID and the update is not; install it from the release page")
	}
	return nil
}

// developerID reports whether path is signed with a Developer ID certificate.
func developerID(path string) bool {
	out, _ := exec.Command("/usr/bin/codesign", "-dv", "--verbose=2", path).CombinedOutput()
	return strings.Contains(string(out), "Authority=Developer ID Application")
}

// install starts the swap helper, which waits for this process to exit.
func (m *macUpdate) install(next string, relaunch bool) error {
	live, err := m.app()
	if err != nil {
		return err
	}
	if _, err := os.Stat(filepath.Join(next, "Contents", "Info.plist")); err != nil {
		return fmt.Errorf("staged update is gone: %w", err)
	}
	r := "0"
	if relaunch {
		r = "1"
	}
	if err := os.MkdirAll(m.s.dir, 0o755); err != nil {
		return err
	}
	logPath := filepath.Join(m.s.dir, "update.log")
	return m.s.launch("/bin/sh", "-c", swapScript, "irori-update", strconv.Itoa(m.pid()), live, next, r, logPath)
}

// cleanup removes a staged bundle an update never installed and an old one
// a swap could not delete.
func (m *macUpdate) cleanup() {
	live, err := m.app()
	if err != nil {
		return
	}
	next, old := sidePaths(live)
	_ = os.RemoveAll(next)
	_ = os.RemoveAll(old)
}

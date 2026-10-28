//go:build darwin

package update

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

// fakeApp builds a minimal Irori.app at version under dir and signs it ad hoc
// with Irori's bundle id, the way release.yml signs an unsigned build. The
// executable is a copy of /usr/bin/true: any Mach-O will do.
func fakeApp(t *testing.T, dir, version string) string {
	t.Helper()
	app := filepath.Join(dir, "Irori.app")
	macos := filepath.Join(app, "Contents", "MacOS")
	if err := os.MkdirAll(macos, 0o755); err != nil {
		t.Fatal(err)
	}
	plist := fmt.Sprintf(`<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key><string>irori</string>
	<key>CFBundleIdentifier</key><string>%s</string>
	<key>CFBundleName</key><string>Irori</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>%s</string>
	<key>CFBundleVersion</key><string>%s</string>
</dict>
</plist>
`, macBundleID, version, version)
	if err := os.WriteFile(filepath.Join(app, "Contents", "Info.plist"), []byte(plist), 0o644); err != nil {
		t.Fatal(err)
	}
	exe, err := os.ReadFile("/usr/bin/true")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(macos, "irori"), exe, 0o755); err != nil {
		t.Fatal(err)
	}
	if out, err := exec.Command("codesign", "--force", "--deep", "--sign", "-", "--identifier", macBundleID, app).CombinedOutput(); err != nil {
		t.Fatalf("codesign: %v: %s", err, out)
	}
	return app
}

// zipApp zips app the way release.yml does (ditto --keepParent).
func zipApp(t *testing.T, app, zip string) []byte {
	t.Helper()
	if out, err := exec.Command("ditto", "-c", "-k", "--keepParent", app, zip).CombinedOutput(); err != nil {
		t.Fatalf("ditto: %v: %s", err, out)
	}
	b, err := os.ReadFile(zip)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func bundleVersion(t *testing.T, app string) string {
	t.Helper()
	v, err := plistString(filepath.Join(app, "Contents", "Info.plist"), "CFBundleShortVersionString")
	if err != nil {
		t.Fatal(err)
	}
	return v
}

// sleeper is a stand-in for the GUI process the swap helper waits on.
func sleeper(t *testing.T) *exec.Cmd {
	t.Helper()
	cmd := exec.Command("/bin/sleep", "60")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cmd.Process.Kill(); _ = cmd.Wait() })
	return cmd
}

// macRelease sets up an installed Irori 1.0.0 under root/Applications and a
// fake GitHub release 1.2.3 whose .app.zip holds the bundle that tweak (if
// set) has altered after signing. It returns the service, wired to the live
// bundle and to gui as the process to wait for, and the live bundle's path.
func macRelease(t *testing.T, version string, tweak func(app string), gui *exec.Cmd) (*UpdateService, string) {
	t.Helper()
	root := t.TempDir()
	apps := filepath.Join(root, "Applications")
	live := fakeApp(t, apps, "1.0.0")
	newApp := fakeApp(t, filepath.Join(root, "build"), version)
	if tweak != nil {
		tweak(newApp)
	}
	asset := "irori-1.2.3-macos-" + runtime.GOARCH + ".app.zip"
	g := &fakeGitHub{tag: "v1.2.3", asset: asset, payload: zipApp(t, newApp, filepath.Join(root, asset))}
	s := testService(t, g.serve(t), "1.0.0")
	pid := os.Getpid
	if gui != nil {
		pid = func() int { return gui.Process.Pid }
	}
	s.plat = macPlatform(s, func() (string, error) { return live, nil }, pid)
	return s, live
}

// The whole macOS path: find the .app.zip, verify it against SHA256SUMS,
// unpack and check it, stage it beside the live bundle, and on quit swap it
// in once the GUI process is gone.
func TestMacUpdateSwapsBundle(t *testing.T) {
	gui := sleeper(t)
	s, live := macRelease(t, "1.2.3", nil, gui)
	st, err := s.CheckAndDownload()
	if err != nil {
		t.Fatal(err)
	}
	next, old := sidePaths(live)
	if st.Phase != PhaseReady || st.InstallerPath != next {
		t.Fatalf("status = %+v, want ready with %s", st, next)
	}
	if v := bundleVersion(t, next); v != "1.2.3" {
		t.Fatalf("staged bundle is %s", v)
	}

	s.InstallOnExit() // quitting: swap without relaunch
	time.Sleep(500 * time.Millisecond)
	if v := bundleVersion(t, live); v != "1.0.0" {
		t.Fatalf("swapped to %s while the GUI still runs", v)
	}
	_ = gui.Process.Kill()
	_ = gui.Wait() // a zombie still answers kill -0

	deadline := time.Now().Add(10 * time.Second)
	for {
		_, errNext := os.Stat(next)
		_, errOld := os.Stat(old)
		if bundleVersion(t, live) == "1.2.3" && os.IsNotExist(errNext) && os.IsNotExist(errOld) {
			break
		}
		if time.Now().After(deadline) {
			log, _ := os.ReadFile(filepath.Join(s.dir, "update.log"))
			t.Fatalf("bundle not swapped (version %s, next %v, old %v); helper log:\n%s",
				bundleVersion(t, live), errNext, errOld, log)
		}
		time.Sleep(100 * time.Millisecond)
	}
	if out, err := exec.Command("codesign", "--verify", "--deep", "--strict", live).CombinedOutput(); err != nil {
		t.Fatalf("installed bundle's signature broke: %v: %s", err, out)
	}
}

// An update whose signature, version or bundle id is wrong never reaches
// "ready", and leaves nothing staged beside the live bundle.
func TestMacUpdateRefusesBadBundle(t *testing.T) {
	cases := []struct {
		name    string
		version string
		tweak   func(app string)
		want    string
	}{
		{"tampered", "1.2.3", func(app string) {
			res := filepath.Join(app, "Contents", "Resources")
			_ = os.MkdirAll(res, 0o755)
			_ = os.WriteFile(filepath.Join(res, "extra.txt"), []byte("added after signing"), 0o644)
		}, "signature"},
		{"wrong version", "1.2.2", nil, "version"},
		{"wrong bundle id", "1.2.3", func(app string) {
			plist := filepath.Join(app, "Contents", "Info.plist")
			_ = exec.Command("plutil", "-replace", "CFBundleIdentifier", "-string", "com.example.other", plist).Run()
			_ = exec.Command("codesign", "--force", "--deep", "--sign", "-", app).Run()
		}, "bundle id"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			s, live := macRelease(t, c.version, c.tweak, nil)
			st, err := s.CheckAndDownload()
			if err == nil || st.Phase != PhaseError || !strings.Contains(err.Error(), c.want) {
				t.Fatalf("status %+v err %v, want an error about %s", st, err, c.want)
			}
			next, _ := sidePaths(live)
			if _, err := os.Stat(next); !os.IsNotExist(err) {
				t.Fatalf("%s left staged (stat err %v)", next, err)
			}
		})
	}
}

// RestartToUpdate passes the helper this process's pid, both bundles and
// relaunch=1; InstallOnExit passes relaunch=0.
func TestMacInstallArgs(t *testing.T) {
	s, live := macRelease(t, "1.2.3", nil, nil)
	if _, err := s.CheckAndDownload(); err != nil {
		t.Fatal(err)
	}
	f := &fakeLauncher{}
	s.launch = f.launch
	if err := s.RestartToUpdate(); err != nil {
		t.Fatal(err)
	}
	next, _ := sidePaths(live)
	if len(f.calls) != 1 {
		t.Fatalf("launches = %v, want one", f.calls)
	}
	c := f.calls[0]
	want := []string{"/bin/sh", "-c", swapScript, "irori-update", fmt.Sprint(os.Getpid()), live, next, "1", filepath.Join(s.dir, "update.log")}
	if strings.Join(c, "\x00") != strings.Join(want, "\x00") {
		t.Fatalf("launch = %q\nwant     %q", c, want)
	}
}

// cleanup at startup removes a staged bundle that was never installed and an
// old one a swap left behind, and nothing else.
func TestMacCleanup(t *testing.T) {
	dir := t.TempDir()
	live := filepath.Join(dir, "Irori.app")
	next, old := sidePaths(live)
	for _, p := range []string{live, next, old, filepath.Join(dir, "Other.app")} {
		if err := os.MkdirAll(filepath.Join(p, "Contents"), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	s := New("x/y", "1.0.0")
	s.plat = macPlatform(s, func() (string, error) { return live, nil }, os.Getpid)
	s.plat.cleanup()
	for _, p := range []string{next, old} {
		if _, err := os.Stat(p); !os.IsNotExist(err) {
			t.Errorf("%s kept", p)
		}
	}
	for _, p := range []string{live, filepath.Join(dir, "Other.app")} {
		if _, err := os.Stat(p); err != nil {
			t.Errorf("%s removed", p)
		}
	}
}

package update

import (
	"path/filepath"
	"testing"
)

func TestAppBundleRoot(t *testing.T) {
	app := filepath.Join("Applications", "Irori.app")
	cases := []struct{ exe, want string }{
		{filepath.Join(app, "Contents", "MacOS", "irori"), app},
		{filepath.Join("bin", "irori"), ""},
		{filepath.Join("x", "Contents", "MacOS", "irori"), ""},
		{filepath.Join(app, "Contents", "Resources", "irori"), ""},
	}
	for _, c := range cases {
		if got := appBundleRoot(c.exe); got != c.want {
			t.Errorf("appBundleRoot(%q) = %q, want %q", c.exe, got, c.want)
		}
	}
	if !translocated("/private/var/folders/x/T/AppTranslocation/ABC/d/Irori.app") || translocated("/Applications/Irori.app") {
		t.Error("translocated misjudges a path")
	}
}

func TestSidePaths(t *testing.T) {
	app := filepath.Join("Applications", "Irori.app")
	next, old := sidePaths(app)
	if next != filepath.Join("Applications", ".Irori.app.new") || old != filepath.Join("Applications", ".Irori.app.old") {
		t.Fatalf("sidePaths = %q, %q", next, old)
	}
}

func TestNumericVersion(t *testing.T) {
	for in, want := range map[string]string{"1.2.3": "1.2.3", "v1.2.3": "1.2.3", "0.4.0-rc.1": "0.4.0", "1.0.0+b7": "1.0.0"} {
		if got := numericVersion(in); got != want {
			t.Errorf("numericVersion(%q) = %q, want %q", in, got, want)
		}
	}
}

// The macOS updater takes the zipped .app for its arch (or a universal one)
// and never the .dmg, the Windows assets, or another arch.
func TestIsMacAppAsset(t *testing.T) {
	cases := []struct {
		name, arch string
		want       bool
	}{
		{"irori-1.2.3-macos-arm64.app.zip", "arm64", true},
		{"irori-1.2.3-macos-universal.app.zip", "arm64", true},
		{"irori-1.2.3-macos-universal.app.zip", "amd64", true},
		{"irori-1.2.3-macos-arm64.app.zip", "amd64", false},
		{"irori-1.2.3-macos-arm64.dmg", "arm64", false},
		{"irori-1.2.3-windows-amd64.zip", "amd64", false},
		{"irori-1.2.3-windows-amd64-installer.exe", "amd64", false},
		{"SHA256SUMS", "arm64", false},
	}
	for _, c := range cases {
		if got := isMacAppAsset(c.name, c.arch); got != c.want {
			t.Errorf("isMacAppAsset(%q, %q) = %v, want %v", c.name, c.arch, got, c.want)
		}
	}
	if isInstallerAsset("irori-1.2.3-macos-arm64.app.zip") {
		t.Error("the Windows updater would take the macOS zip")
	}
}

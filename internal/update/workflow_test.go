package update

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

// TestReleaseWorkflowAssetNames reads .github/workflows/release.yml and checks
// that the Windows installer and the macOS .app.zip it uploads are the assets
// each platform's updater picks, that neither takes the other's assets (or
// the portable zip or the dmg), and that the release carries SHA256SUMS,
// without which the updater offers nothing.
func TestReleaseWorkflowAssetNames(t *testing.T) {
	b, err := os.ReadFile(filepath.Join("..", "..", ".github", "workflows", "release.yml"))
	if err != nil {
		t.Fatal(err)
	}
	wf := string(b)
	name := func(v string) string {
		m := regexp.MustCompile(`\$` + v + `\s*=\s*"([^"]+)"`).FindStringSubmatch(wf)
		if m == nil {
			t.Fatalf("release.yml sets no $%s = \"...\"", v)
		}
		return strings.ReplaceAll(m[1], "$v", "1.2.3")
	}
	installer, zip := name("installer"), name("zip")
	if want := "irori-1.2.3-windows-amd64-installer.exe"; installer != want {
		t.Errorf("installer asset %q, want %q", installer, want)
	}
	if !isInstallerAsset(installer) {
		t.Errorf("findUpdate would not pick %q", installer)
	}
	if isInstallerAsset(zip) {
		t.Errorf("findUpdate would take the zip %q for the installer", zip)
	}

	// The macOS job is bash: appzip="...", dmg="...".
	shName := func(v string) string {
		m := regexp.MustCompile(`(?m)^\s*` + v + `="([^"]+)"`).FindStringSubmatch(wf)
		if m == nil {
			t.Fatalf("release.yml sets no %s=\"...\"", v)
		}
		return strings.ReplaceAll(m[1], "$v", "1.2.3")
	}
	appzip, dmg := shName("appzip"), shName("dmg")
	if want := "irori-1.2.3-macos-arm64.app.zip"; appzip != want {
		t.Errorf("macOS app asset %q, want %q", appzip, want)
	}
	if want := "irori-1.2.3-macos-arm64.dmg"; dmg != want {
		t.Errorf("macOS dmg asset %q, want %q", dmg, want)
	}
	if !isMacAppAsset(appzip, "arm64") {
		t.Errorf("the macOS updater would not pick %q", appzip)
	}
	for _, other := range []string{dmg, installer, zip} {
		if isMacAppAsset(other, "arm64") {
			t.Errorf("the macOS updater would take %q", other)
		}
	}
	for _, other := range []string{appzip, dmg} {
		if isInstallerAsset(other) {
			t.Errorf("the Windows updater would take %q", other)
		}
	}

	if !strings.Contains(wf, "> SHA256SUMS") || !isChecksumAsset("SHA256SUMS") {
		t.Error("release.yml no longer writes SHA256SUMS")
	}
	if Repo != "StephenSHorton/irori" {
		t.Errorf("Repo = %q", Repo)
	}
}

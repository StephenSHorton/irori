package update

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

// TestReleaseWorkflowAssetNames reads .github/workflows/release.yml and checks
// that the installer it uploads is the asset findUpdate picks, that the zip
// is not mistaken for it, and that the release carries SHA256SUMS, without
// which the updater offers nothing.
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
	if !strings.Contains(wf, "> SHA256SUMS") || !isChecksumAsset("SHA256SUMS") {
		t.Error("release.yml no longer writes SHA256SUMS")
	}
	if Repo != "StephenSHorton/irori" {
		t.Errorf("Repo = %q", Repo)
	}
}

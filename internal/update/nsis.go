package update

import (
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

// nsisPlatform updates Windows by running the release's NSIS installer
// silently. It has no build tag, so its tests run on every OS.
func nsisPlatform(s *UpdateService) platform {
	return platform{
		assetKind: "windows installer",
		isAsset:   isInstallerAsset,
		install: func(path string, relaunch bool) error {
			args := []string{"/S"}
			if !relaunch {
				args = append(args, "/NORELAUNCH")
			}
			return s.launch(path, append(args, waitPIDArg())...)
		},
		cleanup: func() {
			if exe, err := os.Executable(); err == nil {
				CleanupOldExes(exe)
			}
		},
	}
}

// waitPIDArg tells the installer which process to wait for before it swaps the
// exe: this one, the GUI. Waiting on the process rather than on the file lock
// matters because `irori mcp` children may hold the lock indefinitely.
func waitPIDArg() string { return "/WAITPID=" + strconv.Itoa(os.Getpid()) }

// CleanupOldExes deletes the copies of exe that the installer renamed aside
// (exe + ".old", ".old1", ...) during an update while `irori mcp` or `irori
// serve` held the old image. A copy still running fails to delete and is left
// for the next start. It returns the paths it removed.
//
//wails:ignore
func CleanupOldExes(exe string) []string {
	dir, prefix := filepath.Dir(exe), strings.ToLower(filepath.Base(exe))+".old"
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil
	}
	var removed []string
	for _, e := range entries {
		if e.IsDir() || !isOldCopy(strings.ToLower(e.Name()), prefix) {
			continue
		}
		p := filepath.Join(dir, e.Name())
		if os.Remove(p) == nil {
			removed = append(removed, p)
		}
	}
	return removed
}

// isOldCopy reports whether name is prefix itself or prefix plus digits, the
// names project.nsi renames the old exe to.
func isOldCopy(name, prefix string) bool {
	rest, ok := strings.CutPrefix(name, prefix)
	if !ok {
		return false
	}
	for _, r := range rest {
		if r < '0' || r > '9' {
			return false
		}
	}
	return true
}

// Package paths says where Irori and Claude Code keep their files on this OS.
package paths

import (
	"os"
	"path/filepath"
	"runtime"
)

// DataDir is Irori's own folder: %LOCALAPPDATA%\irori on Windows, ~/.irori elsewhere.
// IRORI_HOME overrides it (tests, a second instance).
func DataDir() (string, error) {
	if dir := os.Getenv("IRORI_HOME"); dir != "" {
		return dir, nil
	}
	if runtime.GOOS == "windows" {
		if local := os.Getenv("LOCALAPPDATA"); local != "" {
			return filepath.Join(local, "irori"), nil
		}
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, ".irori"), nil
}

// ClaudeDir is Claude Code's config folder: CLAUDE_CONFIG_DIR, else ~/.claude.
func ClaudeDir() (string, error) {
	if dir := os.Getenv("CLAUDE_CONFIG_DIR"); dir != "" {
		return dir, nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, ".claude"), nil
}

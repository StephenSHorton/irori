//go:build !windows

package main

import (
	"os"
	"path/filepath"
)

// downloadsDir is the person's Downloads folder, ~/Downloads.
func downloadsDir() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, "Downloads"), nil
}

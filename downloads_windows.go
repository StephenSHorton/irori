//go:build windows

package main

import (
	"os"
	"path/filepath"

	"golang.org/x/sys/windows"
)

// downloadsDir is the person's Downloads folder: the known folder, which
// the person may have moved, else Downloads in their home.
func downloadsDir() (string, error) {
	if p, err := windows.KnownFolderPath(windows.FOLDERID_Downloads, 0); err == nil && p != "" {
		return p, nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, "Downloads"), nil
}

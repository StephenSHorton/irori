package discovery

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"

	"github.com/StephenSHorton/irori/internal/proc"
)

func newToken() (string, error) {
	buf := make([]byte, 32)
	if _, err := rand.Read(buf); err != nil {
		return "", err
	}
	return hex.EncodeToString(buf), nil
}

// write puts the JSON in a temp file beside the target and renames it over,
// so a reader sees the old file or the new one, never half of one. os.Rename
// replaces an existing file on Windows too (MoveFileEx with REPLACE_EXISTING).
func write(dir string, lf Lockfile) error {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}
	out, err := json.MarshalIndent(lf, "", "  ")
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(dir, FileName+".*.tmp")
	if err != nil {
		return err
	}
	name := tmp.Name()
	defer os.Remove(name) // a no-op once renamed
	// CreateTemp already makes 0600 files on unix; Chmod says so explicitly.
	// Windows only honours the read-only bit, and the data dir is per-user.
	if err := tmp.Chmod(0o600); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(out); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(name, filepath.Join(dir, FileName))
}

// load reads the file without looking at its pid.
func load(dir string) (Lockfile, error) {
	path := filepath.Join(dir, FileName)
	raw, err := os.ReadFile(path)
	if errors.Is(err, fs.ErrNotExist) {
		return Lockfile{}, ErrNotRunning
	}
	if err != nil {
		return Lockfile{}, err
	}
	var lf Lockfile
	if err := json.Unmarshal(raw, &lf); err != nil {
		return Lockfile{}, fmt.Errorf("discovery: %s is unreadable: %w", path, err)
	}
	return lf, nil
}

func read(dir string) (Lockfile, error) {
	lf, err := load(dir)
	if err != nil {
		return Lockfile{}, err
	}
	if !proc.Alive(lf.PID) {
		return Lockfile{}, ErrNotRunning
	}
	return lf, nil
}

func remove(dir string, pid int) error {
	lf, err := load(dir)
	if errors.Is(err, ErrNotRunning) {
		return nil
	}
	if err != nil {
		return err
	}
	if lf.PID != pid {
		return nil
	}
	if err := os.Remove(filepath.Join(dir, FileName)); err != nil && !errors.Is(err, fs.ErrNotExist) {
		return err
	}
	return nil
}

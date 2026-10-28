// Package device is this machine's identity in the Irori mesh.
//
// device.json holds a stable id and a display name, and from v0.4 the
// person the device belongs to and its ed25519 public key. The device's
// secret key is internal/identity's (device.key), never this file's.
package device

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"runtime"
)

// Identity is persisted as device.json in the data dir.
type Identity struct {
	// ID is random and stable for the life of the install, never derived from hardware.
	ID string `json:"id"`
	// Name is a display name; the hostname by default.
	Name string `json:"name"`
	// OS is runtime.GOOS when the identity was created.
	OS string `json:"os"`
	// PersonID is the person this device belongs to (v0.4, design §2).
	PersonID string `json:"personId,omitempty"`
	// DevicePub is the device's ed25519 public key, hex (v0.4).
	DevicePub string `json:"devicePub,omitempty"`
}

// LoadOrCreate reads <dir>/device.json, creating it (0600) on first run.
func LoadOrCreate(dir string) (Identity, error) {
	path := filepath.Join(dir, "device.json")
	raw, err := os.ReadFile(path)
	if err == nil {
		var id Identity
		if err := json.Unmarshal(raw, &id); err != nil || id.ID == "" {
			return Identity{}, fmt.Errorf("device: %s is unreadable; move it aside to make a new identity", path)
		}
		return id, nil
	}
	if !errors.Is(err, fs.ErrNotExist) {
		return Identity{}, err
	}
	buf := make([]byte, 16)
	if _, err := rand.Read(buf); err != nil {
		return Identity{}, err
	}
	name, _ := os.Hostname()
	id := Identity{ID: hex.EncodeToString(buf), Name: name, OS: runtime.GOOS}
	if err := Save(dir, id); err != nil {
		return Identity{}, err
	}
	return id, nil
}

// Save writes id to <dir>/device.json (0600), replacing it atomically.
func Save(dir string, id Identity) error {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}
	out, _ := json.MarshalIndent(id, "", "  ")
	path := filepath.Join(dir, "device.json")
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, out, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

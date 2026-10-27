// Package device is this machine's identity in the Irori mesh.
//
// v0.1 only needs a stable id and a display name. Signing keys arrive with
// irori-net (iroh) in v0.4; the file format leaves room for them.
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
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return Identity{}, err
	}
	out, _ := json.MarshalIndent(id, "", "  ")
	if err := os.WriteFile(path, out, 0o600); err != nil {
		return Identity{}, err
	}
	return id, nil
}

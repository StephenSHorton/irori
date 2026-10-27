// Package discovery is how agents and other Irori processes find the running
// Irori: a lockfile irori.json in paths.DataDir() holding the loopback port
// and a bearer token. No sockets, no fixed port.
//
// CONTRACT (implemented by the discovery/api work package; keep these signatures):
package discovery

import (
	"errors"
	"time"
)

// ErrNotRunning means there is no lockfile, or its process is gone.
var ErrNotRunning = errors.New("irori is not running")

// Lockfile is irori.json. Written atomically with mode 0600.
type Lockfile struct {
	PID       int       `json:"pid"`
	Port      int       `json:"port"`
	Token     string    `json:"token"` // 32 random bytes, hex
	Version   string    `json:"version"`
	DeviceID  string    `json:"deviceId"`
	Headless  bool      `json:"headless"`
	StartedAt time.Time `json:"startedAt"`
}

// FileName is the lockfile's name inside the data dir.
const FileName = "irori.json"

// NewToken returns a fresh random token.
func NewToken() (string, error) { return newToken() }

// Write writes dir/irori.json atomically (temp file + rename), creating dir.
func Write(dir string, lf Lockfile) error { return write(dir, lf) }

// Read reads dir/irori.json. It returns ErrNotRunning when the file is missing
// or its PID is not alive.
func Read(dir string) (Lockfile, error) { return read(dir) }

// Remove deletes dir/irori.json only if it still names pid, so a newer
// instance's lockfile is never removed by an older one exiting.
func Remove(dir string, pid int) error { return remove(dir, pid) }

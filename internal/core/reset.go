package core

import (
	"errors"
	"fmt"
	"io/fs"
	"log"
	"os"
	"path/filepath"
	"time"

	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/pair"
)

// Reset (v0.4 §3) makes this device a new one: its log, device.json and
// device.key go, so it comes back with a new device id and key, its own
// first-run person and nothing of another mesh's. Only the settings stay.
// It is the way back for a device that was removed, signed two histories,
// or is in another person's mesh and should join this one.
//
// Nothing is deleted: the files move into reset-<time> in the data dir.

// ResetFile marks a Reset asked for while Irori runs; the next Start does
// it before it opens anything.
const ResetFile = "reset-pending"

// resetMoves are the data dir entries a Reset moves aside. Everything else
// (settings.json, relay.json, irori.json) stays.
var resetMoves = []string{
	"irori.db", "irori.db-wal", "irori.db-shm", "irori.db.v0.3",
	"device.json", identity.DeviceKeyFile, identity.PersonFile,
	mesh.PeersFile, pair.RowsFile, "capture-cursors.json", "files", "blobs",
}

// RequestReset marks this device for Reset at its next start (the GUI's
// Reset button: the app quits, and opening it again finishes it).
func (c *Core) RequestReset() error {
	return os.WriteFile(filepath.Join(c.opts.DataDir, ResetFile), []byte(time.Now().UTC().Format(time.RFC3339)+"\n"), 0o600)
}

// Reset moves this device's log and identity aside now (`irori pair
// reset`). It refuses while a Core runs on dataDir. It returns the folder
// the files went to.
func Reset(dataDir string) (string, error) {
	if lf, err := discovery.Read(dataDir); err == nil && lf.PID != os.Getpid() {
		return "", fmt.Errorf("%w (pid %d): quit it first, or use Reset in Settings → Devices", ErrAlreadyRunning, lf.PID)
	}
	return resetNow(dataDir)
}

func resetNow(dataDir string) (string, error) {
	dst, err := os.MkdirTemp(dataDir, "reset-"+time.Now().UTC().Format("20060102-150405")+"-")
	if err != nil {
		return "", err
	}
	for _, name := range resetMoves {
		src := filepath.Join(dataDir, name)
		if _, err := os.Lstat(src); errors.Is(err, fs.ErrNotExist) {
			continue
		}
		if err := os.Rename(src, filepath.Join(dst, name)); err != nil {
			return "", fmt.Errorf("reset: move %s: %w", name, err)
		}
	}
	if err := os.Remove(filepath.Join(dataDir, ResetFile)); err != nil && !errors.Is(err, fs.ErrNotExist) {
		return "", err
	}
	return dst, nil
}

// resetIfAsked runs a Reset that RequestReset marked.
func resetIfAsked(dataDir string) error {
	if _, err := os.Stat(filepath.Join(dataDir, ResetFile)); err != nil {
		return nil
	}
	dst, err := resetNow(dataDir)
	if err != nil {
		return err
	}
	log.Printf("reset: this device's log and identity moved to %s; it starts as a new device", dst)
	return nil
}

// Relay is this device's relay setting and how the relay is doing (§4.4).
func (c *Core) Relay() (mesh.Relay, mesh.RelayStatus) {
	r, err := mesh.LoadRelay(c.opts.DataDir)
	if err != nil {
		return mesh.Relay{}, mesh.RelayStatus{State: "error", Detail: err.Error()}
	}
	if rs, ok := c.net.(mesh.RelaySetter); ok {
		return r, rs.RelayStatus()
	}
	if r.Direct() {
		return r, mesh.RelayStatus{State: "none"}
	}
	return r, mesh.RelayStatus{State: "pending"}
}

// SetRelay saves the relay setting (the zero value is Direct only) and
// hands it to the transport for its next connections. GUI only.
func (c *Core) SetRelay(r mesh.Relay) error {
	if err := mesh.SaveRelay(c.opts.DataDir, r); err != nil {
		return err
	}
	if rs, ok := c.net.(mesh.RelaySetter); ok {
		rs.SetRelay(r)
	}
	return nil
}

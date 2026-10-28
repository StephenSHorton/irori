package core

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/notify"
)

// Reset (§3): the next start after RequestReset is a new device with a new
// id and key and its own person; the old log and identity are moved aside,
// not deleted, and the settings stay.
func TestResetMakesANewDevice(t *testing.T) {
	net := mesh.NewFakeNet()
	data := t.TempDir()
	c := startMeshCore(t, net, data)
	oldID, oldPerson := c.device.ID, c.Person().ID
	if err := os.WriteFile(filepath.Join(data, notify.SettingsFile), []byte(`{}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := c.RequestReset(); err != nil {
		t.Fatal(err)
	}
	if _, err := Reset(data); err == nil {
		t.Fatal("Reset ran while a Core holds the data dir")
	}
	c.Close()
	c2 := startMeshCore(t, net, data)
	if c2.device.ID == oldID || c2.Person().ID == oldPerson {
		t.Fatalf("after reset: device %s person %s, the same as before", c2.device.ID, c2.Person().ID)
	}
	if _, err := os.Stat(filepath.Join(data, ResetFile)); !os.IsNotExist(err) {
		t.Fatalf("reset marker left: %v", err)
	}
	if _, err := os.Stat(filepath.Join(data, notify.SettingsFile)); err != nil {
		t.Fatalf("settings did not stay: %v", err)
	}
	moved, _ := filepath.Glob(filepath.Join(data, "reset-*", "irori.db"))
	if len(moved) != 1 {
		t.Fatalf("old log not moved aside: %v", moved)
	}
	c2.Close()
	// The CLI's Reset, with nothing running: moved again.
	if _, err := Reset(data); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(data, "device.json")); !os.IsNotExist(err) {
		t.Fatalf("device.json after Reset: %v", err)
	}
}

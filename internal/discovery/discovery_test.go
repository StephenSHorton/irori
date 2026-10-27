package discovery

import (
	"encoding/hex"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"
	"time"
)

func TestNewToken(t *testing.T) {
	a, err := NewToken()
	if err != nil {
		t.Fatal(err)
	}
	b, _ := NewToken()
	raw, err := hex.DecodeString(a)
	if err != nil || len(raw) != 32 {
		t.Fatalf("token %q is not 32 hex bytes", a)
	}
	if a == b {
		t.Fatal("two tokens are equal")
	}
}

func sample(pid int) Lockfile {
	return Lockfile{
		PID: pid, Port: 4321, Token: "abc", Version: "0.1.0", DeviceID: "dev1",
		StartedAt: time.Date(2026, 1, 2, 3, 4, 5, 0, time.UTC),
	}
}

func TestWriteReadRoundTrip(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "nested", "irori")
	want := sample(os.Getpid())
	if err := Write(dir, want); err != nil {
		t.Fatal(err)
	}
	got, err := Read(dir)
	if err != nil {
		t.Fatal(err)
	}
	if got != want {
		t.Fatalf("got %+v, want %+v", got, want)
	}
	if runtime.GOOS != "windows" {
		fi, err := os.Stat(filepath.Join(dir, FileName))
		if err != nil {
			t.Fatal(err)
		}
		if mode := fi.Mode().Perm(); mode != 0o600 {
			t.Fatalf("mode %v, want 0600", mode)
		}
	}
}

func TestWriteReplacesAndLeavesNoTemp(t *testing.T) {
	dir := t.TempDir()
	first := sample(os.Getpid())
	if err := Write(dir, first); err != nil {
		t.Fatal(err)
	}
	second := first
	second.Port = 9999
	// Renaming over an existing file must work on every OS, Windows included.
	if err := Write(dir, second); err != nil {
		t.Fatal(err)
	}
	got, err := Read(dir)
	if err != nil {
		t.Fatal(err)
	}
	if got.Port != 9999 {
		t.Fatalf("port %d, want 9999", got.Port)
	}
	entries, _ := os.ReadDir(dir)
	if len(entries) != 1 || entries[0].Name() != FileName {
		var names []string
		for _, e := range entries {
			names = append(names, e.Name())
		}
		t.Fatalf("dir holds %v, want only %s", names, FileName)
	}
}

func TestReadMissing(t *testing.T) {
	if _, err := Read(t.TempDir()); !errors.Is(err, ErrNotRunning) {
		t.Fatalf("err %v, want ErrNotRunning", err)
	}
}

// deadPID starts and reaps a child, returning a pid that no longer runs.
func deadPID(t *testing.T) int {
	t.Helper()
	cmd := exec.Command(os.Args[0], "-test.run=^$")
	if err := cmd.Run(); err != nil {
		t.Fatalf("run child: %v", err)
	}
	return cmd.ProcessState.Pid()
}

func TestReadDeadPID(t *testing.T) {
	dir := t.TempDir()
	if err := Write(dir, sample(deadPID(t))); err != nil {
		t.Fatal(err)
	}
	if _, err := Read(dir); !errors.Is(err, ErrNotRunning) {
		t.Fatalf("err %v, want ErrNotRunning", err)
	}
}

func TestReadCorrupt(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, FileName), []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := Read(dir); err == nil || errors.Is(err, ErrNotRunning) {
		t.Fatalf("err %v, want a parse error", err)
	}
}

func TestRemoveOnlyIfOurs(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, FileName)
	if err := Write(dir, sample(os.Getpid())); err != nil {
		t.Fatal(err)
	}
	// An older instance exiting must leave a newer one's lockfile alone.
	if err := Remove(dir, os.Getpid()+1); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); err != nil {
		t.Fatalf("lockfile removed by another pid: %v", err)
	}
	if err := Remove(dir, os.Getpid()); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("lockfile still there: %v", err)
	}
	// Removing again is fine.
	if err := Remove(dir, os.Getpid()); err != nil {
		t.Fatal(err)
	}
}

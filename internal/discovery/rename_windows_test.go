package discovery

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

// A reader holding irori.json open must not make Write fail on Windows.
func TestWriteSurvivesBriefReader(t *testing.T) {
	dir := t.TempDir()
	if err := Write(dir, Lockfile{PID: os.Getpid(), Port: 1}); err != nil {
		t.Fatal(err)
	}
	f, err := os.Open(filepath.Join(dir, FileName))
	if err != nil {
		t.Fatal(err)
	}
	go func() { time.Sleep(50 * time.Millisecond); f.Close() }()
	if err := Write(dir, Lockfile{PID: os.Getpid(), Port: 2}); err != nil {
		t.Fatalf("write while a reader had the file open: %v", err)
	}
	lf, err := Read(dir)
	if err != nil || lf.Port != 2 {
		t.Fatalf("read back %+v, %v", lf, err)
	}
}

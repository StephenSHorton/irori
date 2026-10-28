package main

import (
	"context"
	"io"
	"log"
	"os"
	"os/exec"
	"runtime"
	"testing"
	"time"
)

// TestAttachHelperSleep is the stand-in parent the watchParent test spawns:
// it sleeps for IRORI_ATTACH_SLEEP and exits. It does nothing in a normal run.
func TestAttachHelperSleep(t *testing.T) {
	d, err := time.ParseDuration(os.Getenv("IRORI_ATTACH_SLEEP"))
	if err != nil {
		return
	}
	time.Sleep(d)
	os.Exit(0)
}

// watchParent notices the parent exit at once instead of on its 5 s poll,
// and stops when ctx ends while the parent lives.
func TestWatchParent(t *testing.T) {
	if runtime.GOOS != "windows" && runtime.GOOS != "darwin" {
		t.Skip("here watchParent polls every 5 s")
	}
	logger := log.New(io.Discard, "", 0)
	cases := []struct {
		name     string
		sleep    time.Duration
		cancelAt time.Duration // 0: let the parent exit
		orphaned bool
	}{
		{"parent exits", 300 * time.Millisecond, 0, true},
		{"ctx ends first", time.Hour, 200 * time.Millisecond, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			cmd := exec.Command(os.Args[0], "-test.run=^TestAttachHelperSleep$")
			cmd.Env = append(os.Environ(), "IRORI_ATTACH_SLEEP="+tc.sleep.String())
			if err := cmd.Start(); err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { _ = cmd.Process.Kill(); _ = cmd.Wait() })

			ctx, stop := context.WithCancel(context.Background())
			defer stop()
			orphaned := make(chan struct{})
			done := make(chan struct{})
			go func() {
				defer close(done)
				watchParent(ctx, cmd.Process.Pid, time.Now(), logger, func() { close(orphaned) })
			}()
			if tc.cancelAt > 0 {
				time.AfterFunc(tc.cancelAt, stop)
			}
			select {
			case <-done:
			case <-time.After(3 * time.Second):
				t.Fatal("watchParent still running after 3s")
			}
			select {
			case <-orphaned:
				if !tc.orphaned {
					t.Fatal("canceled as orphaned with the parent alive")
				}
			default:
				if tc.orphaned {
					t.Fatal("returned without canceling")
				}
			}
		})
	}
}

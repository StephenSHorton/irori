package proc

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"testing"
	"time"
)

// TestHelperSleep is the child the wait tests spawn: it sleeps for
// IRORI_PROC_SLEEP and exits. It does nothing in a normal run.
func TestHelperSleep(t *testing.T) {
	d, err := time.ParseDuration(os.Getenv("IRORI_PROC_SLEEP"))
	if err != nil {
		return
	}
	time.Sleep(d)
	os.Exit(0)
}

// child starts a copy of this test binary that sleeps for d.
func child(t *testing.T, d time.Duration) *exec.Cmd {
	t.Helper()
	cmd := exec.Command(os.Args[0], "-test.run=^TestHelperSleep$")
	cmd.Env = append(os.Environ(), "IRORI_PROC_SLEEP="+d.String())
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cmd.Process.Kill(); _ = cmd.Wait() })
	return cmd
}

// waitAsync runs WaitExit and sends its result.
func waitAsync(ctx context.Context, pid int, startedBy time.Time) <-chan error {
	out := make(chan error, 1)
	go func() { out <- WaitExit(ctx, pid, startedBy) }()
	return out
}

func TestWaitExit(t *testing.T) {
	cases := []struct {
		name string
		run  func(t *testing.T) (got <-chan error, want error, within time.Duration)
	}{
		{"exits by itself", func(t *testing.T) (<-chan error, error, time.Duration) {
			c := child(t, 300*time.Millisecond)
			return waitAsync(context.Background(), c.Process.Pid, time.Now()), nil, 3 * time.Second
		}},
		{"killed", func(t *testing.T) (<-chan error, error, time.Duration) {
			c := child(t, time.Hour)
			got := waitAsync(context.Background(), c.Process.Pid, time.Now())
			time.AfterFunc(200*time.Millisecond, func() { _ = c.Process.Kill() })
			return got, nil, 2 * time.Second
		}},
		{"canceled", func(t *testing.T) (<-chan error, error, time.Duration) {
			c := child(t, time.Hour)
			ctx, cancel := context.WithCancel(context.Background())
			t.Cleanup(cancel)
			got := waitAsync(ctx, c.Process.Pid, time.Now())
			time.AfterFunc(200*time.Millisecond, cancel)
			return got, context.Canceled, 2 * time.Second
		}},
		{"reused pid", func(t *testing.T) (<-chan error, error, time.Duration) {
			// The live child started after the bound: it can't be the
			// process we meant, so that one is gone.
			before := time.Now().Add(-time.Minute)
			c := child(t, time.Hour)
			return waitAsync(context.Background(), c.Process.Pid, before), nil, time.Second
		}},
	}
	if err := WaitExit(context.Background(), os.Getpid(), time.Now().Add(-time.Hour)); errors.Is(err, ErrCantWait) {
		t.Skip("WaitExit not supported on this OS")
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, want, within := tc.run(t)
			select {
			case err := <-got:
				if !errors.Is(err, want) && err != want {
					t.Fatalf("err = %v, want %v", err, want)
				}
			case <-time.After(within):
				t.Fatalf("WaitExit still waiting after %v", within)
			}
		})
	}
}

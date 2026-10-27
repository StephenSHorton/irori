package fswatch

import (
	"context"
	"os"
	"path/filepath"
	"sync/atomic"
	"testing"
	"time"

	"golang.org/x/sys/windows"
)

// A watcher that stopped on its own (its directory went away) has closed
// its handles; stopping it again must not signal whatever event object
// Windows handed those handle values to next.
func TestStopAfterSelfCloseSignalsNothing(t *testing.T) {
	for _, how := range []string{"Close", "ctx"} {
		t.Run(how, func(t *testing.T) {
			dir := filepath.Join(t.TempDir(), "w")
			if err := os.Mkdir(dir, 0o755); err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			w, err := Watch(ctx, dir, true, 0)
			if err != nil {
				t.Fatal(err)
			}
			if err := os.RemoveAll(dir); err != nil {
				t.Fatal(err)
			}
			deadline := time.After(3 * time.Second)
		drain:
			for {
				select {
				case _, ok := <-w.Events():
					if !ok {
						break drain
					}
				case <-deadline:
					t.Fatal("watcher on a removed directory didn't stop")
				}
			}
			var probes []windows.Handle
			for range 8 {
				h, err := windows.CreateEvent(nil, 1, 0, nil)
				if err != nil {
					t.Fatal(err)
				}
				defer windows.CloseHandle(h)
				probes = append(probes, h)
			}
			if how == "Close" {
				w.Close()
			} else {
				cancel()
				time.Sleep(100 * time.Millisecond) // let the ctx goroutine run
				w.Close()
			}
			for i, h := range probes {
				if r, _ := windows.WaitForSingleObject(h, 0); r != uint32(windows.WAIT_TIMEOUT) {
					t.Fatalf("probe %d (%#x) was signalled", i, h)
				}
			}
		})
	}
}

// A reader that falls behind is told: the Overflow marker isn't dropped
// along with the events.
func TestSlowReaderGetsOverflow(t *testing.T) {
	in := make(chan Event)
	out := make(chan Event, 4)
	go coalesce(in, out, time.Millisecond, "x", new(atomic.Bool))
	for i := range 8 {
		in <- Event{Path: filepath.Join("x", string(rune('a'+i)))}
	}
	time.Sleep(50 * time.Millisecond) // the flush finds out full
	got := 0
	overflow := false
	deadline := time.After(3 * time.Second)
	for !overflow {
		select {
		case e := <-out:
			got++
			overflow = overflow || e.Overflow
		case <-deadline:
			t.Fatalf("no Overflow after %d events", got)
		}
	}
	close(in)
}

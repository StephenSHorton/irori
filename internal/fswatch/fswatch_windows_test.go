package fswatch

import (
	"fmt"
	"sync/atomic"
	"testing"
	"time"
)

// drain reads out until it has been quiet for a while and reports what came.
func drain(out <-chan Event) (paths []string, overflow bool) {
	for {
		select {
		case e := <-out:
			if e.Overflow {
				overflow = true
			} else {
				paths = append(paths, e.Path)
			}
		case <-time.After(300 * time.Millisecond):
			return paths, overflow
		}
	}
}

func TestLostEventsAlwaysReportOverflow(t *testing.T) {
	for _, tc := range []struct {
		name string
		run  func(in chan Event, lost *atomic.Bool)
	}{
		{"out full at flush", func(in chan Event, lost *atomic.Bool) {
			for i := range 5 {
				in <- Event{Path: fmt.Sprintf(`C:\r\p%d`, i)}
			}
		}},
		{"raw full at send", func(in chan Event, lost *atomic.Bool) {
			// in has room for 8: the ninth send is dropped and must still
			// surface as an Overflow once coalesce drains in.
			for i := range 9 {
				send(in, Event{Path: fmt.Sprintf(`C:\r\q%d`, i)}, lost)
			}
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			in := make(chan Event, 8)
			out := make(chan Event, 2)
			var lost atomic.Bool
			tc.run(in, &lost)
			go coalesce(in, out, time.Millisecond, `C:\r`, &lost)
			defer close(in)
			time.Sleep(100 * time.Millisecond) // let the flush meet a full out
			if _, overflow := drain(out); !overflow {
				t.Fatal("events were lost without an Overflow")
			}
		})
	}
}

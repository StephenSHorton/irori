package fswatch

import (
	"sync/atomic"
	"time"
)

// send never blocks the OS reader: an event that does not fit is dropped and
// marks lost, which coalesce turns into an Overflow.
func send(ch chan<- Event, e Event, lost *atomic.Bool) {
	select {
	case ch <- e:
	default:
		lost.Store(true)
	}
}

// coalesce merges repeats of a path inside debounce and forwards them. Lost
// events (a full out, or a full in seen through inLost) leave a sticky
// Overflow that goes out as soon as out has room, so a reader that falls
// behind always learns it must rescan.
func coalesce(in <-chan Event, out chan<- Event, debounce time.Duration, root string, inLost *atomic.Bool) {
	defer close(out)
	pending := map[string]Event{}
	var timer <-chan time.Time
	lost := false
	overflow := Event{Path: root, Overflow: true}
	flush := func() {
		for p, e := range pending {
			select {
			case out <- e:
			default:
				lost = true
			}
			delete(pending, p)
		}
		timer = nil
	}
	for {
		if inLost.Swap(false) {
			lost = true
		}
		var ovOut chan<- Event // nil (never ready) unless an Overflow is owed
		if lost {
			ovOut = out
		}
		select {
		case e, ok := <-in:
			if !ok {
				flush()
				if lost || inLost.Load() {
					select {
					case out <- overflow:
					default:
					}
				}
				return
			}
			if e.Overflow {
				lost = true
				continue
			}
			pending[e.Path] = e
			if timer == nil {
				timer = time.After(debounce)
			}
		case <-timer:
			flush()
		case ovOut <- overflow:
			lost = false
		}
	}
}

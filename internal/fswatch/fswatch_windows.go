package fswatch

import (
	"context"
	"path/filepath"
	"sync"
	"sync/atomic"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"
)

const bufSize = 64 << 10 // the largest buffer ReadDirectoryChangesW takes over a network share; plenty locally

const notifyMask = windows.FILE_NOTIFY_CHANGE_FILE_NAME |
	windows.FILE_NOTIFY_CHANGE_DIR_NAME |
	windows.FILE_NOTIFY_CHANGE_SIZE |
	windows.FILE_NOTIFY_CHANGE_LAST_WRITE

func watch(ctx context.Context, root string, recursive bool, debounce time.Duration) (*Watcher, error) {
	root, err := filepath.Abs(root)
	if err != nil {
		return nil, err
	}
	p, err := windows.UTF16PtrFromString(root)
	if err != nil {
		return nil, err
	}
	h, err := windows.CreateFile(p, windows.FILE_LIST_DIRECTORY,
		windows.FILE_SHARE_READ|windows.FILE_SHARE_WRITE|windows.FILE_SHARE_DELETE,
		nil, windows.OPEN_EXISTING,
		windows.FILE_FLAG_BACKUP_SEMANTICS|windows.FILE_FLAG_OVERLAPPED, 0)
	if err != nil {
		return nil, err
	}
	ioEv, err := windows.CreateEvent(nil, 1, 0, nil)
	if err != nil {
		windows.CloseHandle(h)
		return nil, err
	}
	stopEv, err := windows.CreateEvent(nil, 1, 0, nil)
	if err != nil {
		windows.CloseHandle(ioEv)
		windows.CloseHandle(h)
		return nil, err
	}
	raw := make(chan Event, 1024)
	out := make(chan Event, 1024)
	var once sync.Once
	done := make(chan struct{})
	stop := func() {
		once.Do(func() { windows.SetEvent(stopEv) })
		<-done
	}
	go func() {
		select {
		case <-ctx.Done():
			once.Do(func() { windows.SetEvent(stopEv) })
		case <-done:
		}
	}()
	buf := make([]byte, bufSize)
	var ov windows.Overlapped
	ov.HEvent = ioEv
	var n uint32
	// Arm the first read before returning: changes are only buffered on the
	// handle once a read has been issued, so a write right after Watch
	// returns would otherwise be missed.
	arm := func() error {
		ov = windows.Overlapped{HEvent: ioEv}
		windows.ResetEvent(ioEv)
		err := windows.ReadDirectoryChanges(h, &buf[0], uint32(len(buf)), recursive, notifyMask, &n, &ov, 0)
		if err != nil && err != windows.ERROR_IO_PENDING {
			return err
		}
		return nil
	}
	if err := arm(); err != nil {
		windows.CloseHandle(stopEv)
		windows.CloseHandle(ioEv)
		windows.CloseHandle(h)
		return nil, err
	}
	var rawLost atomic.Bool
	go coalesce(raw, out, debounce, root, &rawLost)
	go func() {
		defer close(done)
		defer close(raw)
		defer windows.CloseHandle(stopEv)
		defer windows.CloseHandle(ioEv)
		defer windows.CloseHandle(h)
		// Use up once before stopEv closes, so a later Close or ctx cancel
		// can't SetEvent a handle value Windows has since given to someone
		// else.
		defer once.Do(func() {})
		for {
			w, _ := windows.WaitForMultipleObjects([]windows.Handle{ioEv, stopEv}, false, windows.INFINITE)
			if w != windows.WAIT_OBJECT_0 {
				windows.CancelIoEx(h, &ov)
				windows.GetOverlappedResult(h, &ov, &n, true)
				return
			}
			err := windows.GetOverlappedResult(h, &ov, &n, false)
			switch {
			case err == windows.ERROR_NOTIFY_ENUM_DIR || err == nil && n == 0:
				// The buffer overflowed: changes were lost.
				send(raw, Event{Path: root, Overflow: true}, &rawLost)
			case err != nil:
				send(raw, Event{Path: root, Overflow: true}, &rawLost)
				return
			default:
				parse(buf[:n], root, raw, &rawLost)
			}
			if err := arm(); err != nil {
				send(raw, Event{Path: root, Overflow: true}, &rawLost)
				return
			}
		}
	}()
	return &Watcher{events: out, stop: stop}, nil
}

// parse walks FILE_NOTIFY_INFORMATION records.
func parse(b []byte, root string, raw chan<- Event, lost *atomic.Bool) {
	for off := 0; off+12 <= len(b); {
		next := *(*uint32)(unsafe.Pointer(&b[off]))
		nameLen := *(*uint32)(unsafe.Pointer(&b[off+8]))
		start := off + 12
		end := start + int(nameLen)
		if end > len(b) {
			return
		}
		name := windows.UTF16ToString(unsafe.Slice((*uint16)(unsafe.Pointer(&b[start])), nameLen/2))
		send(raw, Event{Path: filepath.Join(root, name)}, lost)
		if next == 0 {
			return
		}
		off += int(next)
	}
}

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

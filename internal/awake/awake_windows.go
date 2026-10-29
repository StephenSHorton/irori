//go:build windows

package awake

import (
	"runtime"
	"sync"

	"golang.org/x/sys/windows"
)

// ES_* flags for SetThreadExecutionState.
const (
	esSystemRequired = 0x00000001
	esContinuous     = 0x80000000
)

var procSetThreadExecutionState = windows.NewLazySystemDLL("kernel32.dll").NewProc("SetThreadExecutionState")

// The execution state belongs to the thread that set it, and Go moves
// goroutines between threads, so each hold runs on one goroutine locked to
// its OS thread for the hold's lifetime and clears the state on that same
// thread.
type windowsHold struct {
	once sync.Once
	stop chan struct{}
	done chan struct{}
}

func acquire() (Hold, error) {
	h := &windowsHold{stop: make(chan struct{}), done: make(chan struct{})}
	errc := make(chan error, 1)
	go func() {
		defer close(h.done)
		runtime.LockOSThread()
		defer runtime.UnlockOSThread()
		if r, _, err := procSetThreadExecutionState.Call(uintptr(esContinuous | esSystemRequired)); r == 0 {
			if err == nil || err == windows.ERROR_SUCCESS {
				err = windows.ERROR_INVALID_PARAMETER
			}
			errc <- err
			return
		}
		errc <- nil
		<-h.stop
		_, _, _ = procSetThreadExecutionState.Call(uintptr(esContinuous))
	}()
	if err := <-errc; err != nil {
		<-h.done
		return nil, err
	}
	return h, nil
}

func (h *windowsHold) Release() {
	h.once.Do(func() { close(h.stop) })
	<-h.done
}

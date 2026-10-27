package presence

import (
	"sync"

	"golang.org/x/sys/windows"
)

// watchExit calls onExit once pid's process ends. stop ends the wait (without
// calling onExit) and waits for it. ok is false when the process can't be
// opened (gone already, or another user's); the safety rescan covers it.
func watchExit(pid int, onExit func()) (stop func(), ok bool) {
	h, err := windows.OpenProcess(windows.SYNCHRONIZE, false, uint32(pid))
	if err != nil {
		return nil, false
	}
	stopEv, err := windows.CreateEvent(nil, 1, 0, nil)
	if err != nil {
		windows.CloseHandle(h)
		return nil, false
	}
	done := make(chan struct{})
	go func() {
		defer close(done)
		defer windows.CloseHandle(h)
		w, _ := windows.WaitForMultipleObjects([]windows.Handle{h, stopEv}, false, windows.INFINITE)
		if w == windows.WAIT_OBJECT_0 {
			onExit()
		}
	}()
	var once sync.Once
	return func() {
		once.Do(func() {
			windows.SetEvent(stopEv)
			<-done
			windows.CloseHandle(stopEv)
		})
	}, true
}

package proc

import (
	"context"
	"time"

	"golang.org/x/sys/windows"
)

// waitExit waits on the process handle and a stop event that ctx sets.
func waitExit(ctx context.Context, pid int, startedBy time.Time) error {
	h, err := windows.OpenProcess(windows.SYNCHRONIZE|windows.PROCESS_QUERY_LIMITED_INFORMATION, false, uint32(pid))
	if err != nil {
		if err == windows.ERROR_INVALID_PARAMETER { // no process has this pid
			return nil
		}
		return ErrCantWait
	}
	defer windows.CloseHandle(h)
	if !startedBy.IsZero() {
		var created, exited, kernel, user windows.Filetime
		if windows.GetProcessTimes(h, &created, &exited, &kernel, &user) == nil &&
			time.Unix(0, created.Nanoseconds()).After(startedBy) {
			return nil // pid reused: the process we meant is gone
		}
	}
	stop, err := windows.CreateEvent(nil, 1, 0, nil)
	if err != nil {
		return ErrCantWait
	}
	defer windows.CloseHandle(stop)
	done := make(chan struct{})
	stopped := make(chan struct{})
	go func() {
		defer close(stopped)
		select {
		case <-ctx.Done():
			windows.SetEvent(stop)
		case <-done:
		}
	}()
	r, err := windows.WaitForMultipleObjects([]windows.Handle{h, stop}, false, windows.INFINITE)
	close(done)
	<-stopped // stop stays open until the goroutine is done with it
	switch {
	case err != nil:
		return ErrCantWait
	case r == windows.WAIT_OBJECT_0:
		return nil
	}
	return ctx.Err()
}

package proc

import (
	"context"
	"errors"
	"time"

	"golang.org/x/sys/unix"
)

// wakeIdent is the EVFILT_USER event that ends a wait when ctx does.
const wakeIdent = 1

// waitExit waits on a kqueue for EVFILT_PROC NOTE_EXIT on pid, with an
// EVFILT_USER event that ctx triggers. Unlike a process handle on Windows,
// a kqueue registration is by pid, so the start-time check runs after it:
// a process at pid that started after startedBy is a reuse, and the one we
// meant is gone.
func waitExit(ctx context.Context, pid int, startedBy time.Time) error {
	kq, err := unix.Kqueue()
	if err != nil {
		return ErrCantWait
	}
	defer unix.Close(kq)

	var wake unix.Kevent_t
	unix.SetKevent(&wake, wakeIdent, unix.EVFILT_USER, unix.EV_ADD|unix.EV_CLEAR)
	if _, err := unix.Kevent(kq, []unix.Kevent_t{wake}, nil, nil); err != nil {
		return ErrCantWait
	}
	var exit unix.Kevent_t
	unix.SetKevent(&exit, pid, unix.EVFILT_PROC, unix.EV_ADD|unix.EV_ONESHOT)
	exit.Fflags = unix.NOTE_EXIT
	if _, err := unix.Kevent(kq, []unix.Kevent_t{exit}, nil, nil); err != nil {
		if errors.Is(err, unix.ESRCH) { // no process has this pid
			return nil
		}
		return ErrCantWait
	}
	if !startedBy.IsZero() {
		if st, ok := startTime(pid); ok && st.After(startedBy) {
			return nil // pid reused: the process we meant is gone
		}
	}

	done := make(chan struct{})
	stopped := make(chan struct{})
	go func() {
		defer close(stopped)
		select {
		case <-ctx.Done():
			var trig unix.Kevent_t
			unix.SetKevent(&trig, wakeIdent, unix.EVFILT_USER, 0)
			trig.Fflags = unix.NOTE_TRIGGER
			_, _ = unix.Kevent(kq, []unix.Kevent_t{trig}, nil, nil)
		case <-done:
		}
	}()
	defer func() {
		close(done)
		<-stopped // kq stays open until the goroutine is done with it
	}()

	events := make([]unix.Kevent_t, 2)
	for {
		n, err := unix.Kevent(kq, nil, events, nil)
		if errors.Is(err, unix.EINTR) {
			continue
		}
		if err != nil {
			return ErrCantWait
		}
		for _, ev := range events[:n] {
			if ev.Filter == unix.EVFILT_PROC {
				return nil
			}
		}
		if ctx.Err() != nil {
			return ctx.Err()
		}
	}
}

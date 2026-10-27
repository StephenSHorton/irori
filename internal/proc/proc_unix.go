//go:build !windows

package proc

import (
	"errors"
	"syscall"
	"time"
)

func alive(pid int) bool {
	err := syscall.Kill(pid, 0)
	return err == nil || errors.Is(err, syscall.EPERM)
}

// startTime is not implemented off Windows yet; callers fall back to Alive.
func startTime(int) (time.Time, bool) { return time.Time{}, false }

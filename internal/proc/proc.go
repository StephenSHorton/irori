// Package proc answers "is this process still the one I think it is?"
package proc

import "time"

// Alive reports whether a process with pid exists (a process we may not
// signal still counts as existing; so does a zombie, except on macOS, where
// an exited process waiting for its parent to reap it counts as gone).
func Alive(pid int) bool {
	if pid <= 0 {
		return false
	}
	return alive(pid)
}

// StartTime is when pid started, if the OS can tell us cheaply. ok is false
// when it can't (the process is gone, or the platform isn't supported yet).
func StartTime(pid int) (t time.Time, ok bool) {
	if pid <= 0 {
		return time.Time{}, false
	}
	return startTime(pid)
}

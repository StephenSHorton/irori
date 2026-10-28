package proc

import (
	"errors"
	"syscall"
	"time"

	"golang.org/x/sys/unix"
)

func alive(pid int) bool {
	err := syscall.Kill(pid, 0)
	return err == nil || errors.Is(err, syscall.EPERM)
}

// startTime reads p_starttime from the kernel's process table
// (sysctl kern.proc.pid.<pid>), which works for any user's process.
func startTime(pid int) (time.Time, bool) {
	kp, err := unix.SysctlKinfoProc("kern.proc.pid", pid)
	if err != nil || int(kp.Proc.P_pid) != pid {
		return time.Time{}, false
	}
	tv := kp.Proc.P_starttime
	if tv.Sec == 0 && tv.Usec == 0 {
		return time.Time{}, false
	}
	return time.Unix(int64(tv.Sec), int64(tv.Usec)*1000), true
}

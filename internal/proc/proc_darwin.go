package proc

import (
	"errors"
	"syscall"
	"time"

	"golang.org/x/sys/unix"
)

// sZomb is p_stat for a process that has exited and not been reaped yet
// (SZOMB in <sys/proc.h>).
const sZomb = 5

// alive is kill(pid, 0), except that a zombie counts as gone: it has exited,
// and only its parent has yet to collect it. (The proc.Alive doc's "a zombie
// still counts" is Windows' and Linux's behaviour; on macOS the exit wait
// and the liveness check must agree, or a killed agent would linger on the
// roster until its parent reaped it.)
func alive(pid int) bool {
	err := syscall.Kill(pid, 0)
	if err != nil && !errors.Is(err, syscall.EPERM) {
		return false
	}
	if kp, err := unix.SysctlKinfoProc("kern.proc.pid", pid); err == nil && int(kp.Proc.P_pid) == pid && kp.Proc.P_stat == sZomb {
		return false
	}
	return true
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

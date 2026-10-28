package presence

import (
	"time"

	"github.com/StephenSHorton/irori/internal/proc"
)

// procStartSlack is how far our start time may drift from Claude's procStart
// and still be the same process. lstart has one-second resolution and ours
// is truncated to the second, so 2 s leaves room for both.
const procStartSlack = 2 * time.Second

// maxUnixProcStart: readSession stores an lstart procStart as Unix seconds.
// Anything larger (a FILETIME, some other unit) is not one we can check, so
// it is trusted the way an absent procStart is.
const maxUnixProcStart = 1 << 36

// live reports whether pid is running and, when the session recorded a
// procStart we can read, is the same process that wrote it (not a reused pid).
func live(pid int, procStart uint64) bool {
	if !proc.Alive(pid) {
		return false
	}
	if procStart == 0 || procStart > maxUnixProcStart {
		return true
	}
	st, ok := proc.StartTime(pid)
	if !ok {
		return true
	}
	diff := st.Truncate(time.Second).Sub(time.Unix(int64(procStart), 0))
	if diff < 0 {
		diff = -diff
	}
	return diff <= procStartSlack
}

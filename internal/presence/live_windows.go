package presence

import "github.com/StephenSHorton/irori/internal/proc"

// procStartSlack is how far our FILETIME may drift from Claude's procStart
// and still be the same process: one second.
const procStartSlack = 10_000_000

// live reports whether pid is running and, when the session recorded a
// procStart, is the same process that wrote it (not a reused pid).
func live(pid int, procStart uint64) bool {
	if !proc.Alive(pid) {
		return false
	}
	if procStart == 0 {
		return true
	}
	st, ok := proc.StartTime(pid)
	if !ok {
		// Alive but unreadable (another user's process): trust Alive.
		return true
	}
	ticks := proc.FiletimeTicks(st)
	diff := ticks - procStart
	if procStart > ticks {
		diff = procStart - ticks
	}
	return diff <= procStartSlack
}

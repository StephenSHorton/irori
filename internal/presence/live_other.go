//go:build !windows

package presence

import "github.com/StephenSHorton/irori/internal/proc"

// live reports whether pid is running. procStart's unit off Windows is not
// known yet, so pid reuse goes undetected here.
func live(pid int, _ uint64) bool { return proc.Alive(pid) }

package core

import (
	"os"
	"strconv"

	"github.com/StephenSHorton/irori/internal/proc"
)

// procStartField is this process's start as Claude Code writes procStart.
func procStartField() string {
	st, ok := proc.StartTime(os.Getpid())
	if !ok {
		return ""
	}
	return strconv.FormatUint(proc.FiletimeTicks(st), 10)
}

package proc

import (
	"os"
	"strconv"
	"testing"
)

// procStart in Claude Code's session files is FILETIME ticks; ours must match
// to within a second for pid-reuse detection to work.
func TestFiletimeMatchesClaudeUnits(t *testing.T) {
	st, ok := StartTime(os.Getpid())
	if !ok {
		t.Fatal("no start time on windows")
	}
	ticks := FiletimeTicks(st)
	if s := strconv.FormatUint(ticks, 10); len(s) != 18 {
		t.Fatalf("want 18-digit FILETIME like Claude's procStart, got %s", s)
	}
}

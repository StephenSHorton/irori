package presence

import (
	"os"
	"strconv"
	"testing"

	"github.com/StephenSHorton/irori/internal/proc"
)

// A live pid whose procStart doesn't match is a reused pid, not the session.
func TestReusedPIDNotListed(t *testing.T) {
	st, ok := proc.StartTime(os.Getpid())
	if !ok {
		t.Fatal("no start time for our own process")
	}
	ticks := proc.FiletimeTicks(st)

	dir := t.TempDir()
	writeSession(t, dir, map[string]any{"pid": os.Getpid(), "sessionId": "reused", "cwd": dir,
		"procStart": strconv.FormatUint(ticks-5*procStartSlack, 10)})
	w, _ := testWatcher(dir, false)
	w.impl.scan()
	if cards := w.Snapshot(); len(cards) != 0 {
		t.Fatalf("reused pid listed: %+v", cards)
	}

	writeSession(t, dir, map[string]any{"pid": os.Getpid(), "sessionId": "mine", "cwd": dir,
		"procStart": strconv.FormatUint(ticks+procStartSlack/2, 10)})
	w.impl.scan()
	if cards := w.Snapshot(); len(cards) != 1 || cards[0].SessionID != "mine" {
		t.Fatalf("matching procStart not listed: %+v", cards)
	}
}

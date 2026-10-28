package presence

import (
	"os"
	"os/exec"
	"strconv"
	"strings"
	"testing"
	"time"
)

// lstart is what Claude Code writes as procStart off Windows: ps's lstart
// for pid, in the C locale and UTC.
func lstart(t *testing.T, pid int) string {
	t.Helper()
	cmd := exec.Command("ps", "-o", "lstart=", "-p", strconv.Itoa(pid))
	cmd.Env = append(os.Environ(), "LC_ALL=C", "TZ=UTC")
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("ps: %v", err)
	}
	return strings.TrimSpace(string(out))
}

// readSession understands the procStart ps writes, and live matches it to
// the running process: a matching one is listed, a reused pid is not.
func TestLstartProcStart(t *testing.T) {
	v := lstart(t, os.Getpid())
	got, ok := parseLstart(v)
	if !ok {
		t.Fatalf("parseLstart(%q) failed", v)
	}
	if !live(os.Getpid(), uint64(got.Unix())) {
		t.Fatalf("our own lstart %q (%v) does not match our start time", v, got)
	}

	dir := t.TempDir()
	stale := got.Add(-time.Minute).Format(lstartLayout)
	writeSession(t, dir, map[string]any{"pid": os.Getpid(), "sessionId": "reused", "cwd": dir, "procStart": stale})
	w, _ := testWatcher(dir, false)
	w.impl.scan()
	if cards := w.Snapshot(); len(cards) != 0 {
		t.Fatalf("reused pid listed: %+v", cards)
	}

	writeSession(t, dir, map[string]any{"pid": os.Getpid(), "sessionId": "mine", "cwd": dir, "procStart": v})
	w.impl.scan()
	if cards := w.Snapshot(); len(cards) != 1 || cards[0].SessionID != "mine" {
		t.Fatalf("matching procStart not listed: %+v", cards)
	}
}

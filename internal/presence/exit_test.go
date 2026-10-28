//go:build windows || darwin

package presence

import (
	"os"
	"os/exec"
	"testing"
	"time"
)

// TestHelperSleeper is the child process for TestExitRemovesAtOnce.
func TestHelperSleeper(t *testing.T) {
	if os.Getenv("IRORI_PRESENCE_SLEEPER") != "1" {
		t.Skip("helper process")
	}
	time.Sleep(time.Minute)
}

// An agent killed without removing its session file leaves the roster when
// its process ends, not at the next safety rescan.
func TestExitRemovesAtOnce(t *testing.T) {
	cmd := exec.Command(os.Args[0], "-test.run=^TestHelperSleeper$")
	cmd.Env = append(os.Environ(), "IRORI_PRESENCE_SLEEPER=1")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	defer cmd.Wait()
	defer cmd.Process.Kill()

	dir := t.TempDir()
	writeSession(t, dir, map[string]any{"pid": cmd.Process.Pid, "sessionId": "killed", "cwd": dir})
	w, _ := testWatcher(dir, false)
	w.impl.opts.Interval = time.Hour
	w.impl.opts.SafetyInterval = time.Hour
	ch, _ := runWatcher(t, w)
	await(t, ch, "Added", func(c Change) bool { return c.Kind == Added && c.Card.SessionID == "killed" })
	if err := cmd.Process.Kill(); err != nil {
		t.Fatal(err)
	}
	await(t, ch, "Removed", func(c Change) bool { return c.Kind == Removed && c.Card.SessionID == "killed" })
}

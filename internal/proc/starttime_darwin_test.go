package proc

import (
	"os"
	"os/exec"
	"testing"
	"time"
)

// A child's start time is after ours and close to when we started it.
func TestChildStartTime(t *testing.T) {
	before := time.Now().Add(-time.Second)
	cmd := exec.Command("/bin/sleep", "5")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cmd.Process.Kill(); _ = cmd.Wait() })
	st, ok := StartTime(cmd.Process.Pid)
	if !ok {
		t.Fatal("no start time for a live child")
	}
	if st.Before(before) || st.After(time.Now().Add(time.Second)) {
		t.Fatalf("child start %v outside [%v, now]", st, before)
	}
	self, ok := StartTime(os.Getpid())
	if !ok || !self.Before(st) {
		t.Fatalf("own start %v (ok %v) not before the child's %v", self, ok, st)
	}
}

// A pid with no process has no start time.
func TestNoStartTimeForMissingPid(t *testing.T) {
	cmd := exec.Command("/usr/bin/true")
	if err := cmd.Run(); err != nil {
		t.Fatal(err)
	}
	if _, ok := StartTime(cmd.Process.Pid); ok {
		t.Fatal("start time for a reaped pid")
	}
}

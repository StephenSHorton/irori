//go:build windows

package awake

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"unsafe"

	"golang.org/x/sys/windows"
)

var procCallNtPowerInformation = windows.NewLazySystemDLL("powrprof.dll").NewProc("CallNtPowerInformation")

// systemExecutionState is the system's execution state: ES_SYSTEM_REQUIRED
// is set while any thread on the PC holds it.
func systemExecutionState(t *testing.T) uint32 {
	t.Helper()
	const systemExecutionStateLevel = 16
	var st uint32
	r, _, _ := procCallNtPowerInformation.Call(systemExecutionStateLevel, 0, 0, uintptr(unsafe.Pointer(&st)), unsafe.Sizeof(st))
	if r != 0 {
		t.Fatalf("CallNtPowerInformation: 0x%x", r)
	}
	return st
}

// powercfgSystem is the SYSTEM section of powercfg /requests; ok is false
// where it needs an elevated prompt (it does on Windows 11).
func powercfgSystem(t *testing.T) (string, bool) {
	t.Helper()
	out, err := exec.Command("powercfg", "/requests").CombinedOutput()
	text := string(out)
	if err != nil {
		if strings.Contains(strings.ToLower(text), "administrator") {
			return "", false
		}
		t.Fatalf("powercfg /requests: %v: %s", err, text)
	}
	text = strings.ReplaceAll(text, "\r\n", "\n")
	i := strings.Index(text, "SYSTEM:\n")
	if i < 0 {
		t.Fatalf("no SYSTEM section in powercfg /requests:\n%s", text)
	}
	sec := text[i+len("SYSTEM:\n"):]
	if j := strings.Index(sec, "\n\n"); j >= 0 {
		sec = sec[:j]
	}
	return sec, true
}

// A real hold sets ES_SYSTEM_REQUIRED on the system, and powercfg (when
// this runs elevated) lists this test binary under SYSTEM until it is
// released.
func TestRealHold(t *testing.T) {
	exe, _ := os.Executable()
	name := strings.ToLower(filepath.Base(exe))
	before := systemExecutionState(t)
	h, err := System().Acquire()
	if err != nil {
		t.Fatal(err)
	}
	if st := systemExecutionState(t); st&esSystemRequired == 0 {
		h.Release()
		t.Fatalf("system execution state 0x%x while held: no ES_SYSTEM_REQUIRED", st)
	}
	sec, ok := powercfgSystem(t)
	if ok {
		t.Logf("powercfg /requests SYSTEM while held:\n%s", sec)
		if !strings.Contains(strings.ToLower(sec), name) {
			h.Release()
			t.Fatalf("%s is not under SYSTEM while held:\n%s", name, sec)
		}
	} else {
		t.Logf("powercfg /requests needs an elevated prompt here; checked the system execution state instead")
	}
	h.Release()
	h.Release() // a second release does nothing
	after := systemExecutionState(t)
	t.Logf("system execution state: 0x%x before, 0x%x after release", before, after)
	if before&esSystemRequired == 0 && after&esSystemRequired != 0 {
		t.Fatalf("ES_SYSTEM_REQUIRED still set after release (0x%x)", after)
	}
	if sec, ok := powercfgSystem(t); ok {
		t.Logf("powercfg /requests SYSTEM after release:\n%s", sec)
		if strings.Contains(strings.ToLower(sec), name) {
			t.Fatalf("%s still under SYSTEM after release:\n%s", name, sec)
		}
	}
}

// Holds are per thread, so two at once are two threads, each released on
// its own.
func TestTwoHolds(t *testing.T) {
	a, err := System().Acquire()
	if err != nil {
		t.Fatal(err)
	}
	b, err := System().Acquire()
	if err != nil {
		a.Release()
		t.Fatal(err)
	}
	a.Release()
	if st := systemExecutionState(t); st&esSystemRequired == 0 {
		b.Release()
		t.Fatal(errors.New("releasing one hold dropped the other"))
	}
	b.Release()
}

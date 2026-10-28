package diag

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

// The build Irori ships must not use a toolchain with the Windows
// WSARecvFrom stack bug; this fails the local checks if one is in use.
func TestToolchainNotBad(t *testing.T) {
	if BadToolchain(runtime.Version()) {
		t.Fatalf("%s corrupts goroutine stacks on Windows UDP receives; use go1.26.2 or later (go.mod pins it)", runtime.Version())
	}
}

func TestBadToolchain(t *testing.T) {
	for v, want := range map[string]bool{
		"go1.26.0": true, "go1.26.1": true,
		"go1.25.9": true, "go1.26rc2": true, "go1.21": true,
		"go1.26.2": false, "go1.26.5": false, "go1.27.1": false, "go1.27rc1": false, "devel go1.27-abc": false,
	} {
		if got := BadToolchain(v); got != want {
			t.Errorf("BadToolchain(%q) = %v, want %v", v, got, want)
		}
	}
}

func TestStartLine(t *testing.T) {
	s := StartLine("gui", "0.4.2")
	for _, want := range []string{"irori 0.4.2 start", "role=gui", runtime.Version(), runtime.GOOS + "/" + runtime.GOARCH} {
		if !strings.Contains(s, want) {
			t.Errorf("StartLine = %q, missing %q", s, want)
		}
	}
	if !strings.HasSuffix(s, "\n") {
		t.Errorf("StartLine = %q, want a trailing newline", s)
	}
}

func TestLogRotates(t *testing.T) {
	path := filepath.Join(t.TempDir(), "sub", "irori.log")
	l, err := openLog(path, 100, 2)
	if err != nil {
		t.Fatal(err)
	}
	line := []byte(strings.Repeat("x", 39) + "\n") // 40 bytes: two fit in 100, a third rotates
	for i := 0; i < 10; i++ {
		if _, err := l.Write(line); err != nil {
			t.Fatal(err)
		}
	}
	if err := l.Close(); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{path, path + ".1", path + ".2"} {
		fi, err := os.Stat(name)
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		if fi.Size() == 0 || fi.Size() > 100 {
			t.Errorf("%s is %d bytes, want 1..100", name, fi.Size())
		}
	}
	if _, err := os.Stat(path + ".3"); !os.IsNotExist(err) {
		t.Errorf("%s.3 exists, want only keep=2 old files", path)
	}
}

func TestLogRotatesBigFileAtOpen(t *testing.T) {
	path := filepath.Join(t.TempDir(), "irori.log")
	if err := os.WriteFile(path, make([]byte, 200), 0o600); err != nil {
		t.Fatal(err)
	}
	l, err := openLog(path, 100, 3)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := l.Write([]byte("new\n")); err != nil {
		t.Fatal(err)
	}
	_ = l.Close()
	if b, _ := os.ReadFile(path); string(b) != "new\n" {
		t.Errorf("%s = %q, want a fresh file", path, b)
	}
	if fi, err := os.Stat(path + ".1"); err != nil || fi.Size() != 200 {
		t.Errorf("%s.1: want the old 200 bytes, got %v %v", path, fi, err)
	}
}

const crashChildEnv = "IRORI_DIAG_CRASH_CHILD"

// A panic nobody recovers must land in crash.log, not only on a stderr a GUI
// app does not have.
func TestCrashOutputLandsInFile(t *testing.T) {
	if dir := os.Getenv(crashChildEnv); dir != "" {
		f, err := InstallCrashOutput(dir)
		if err != nil {
			os.Exit(3)
		}
		_, _ = f.WriteString(StartLine("test", "dev"))
		panic("diag test crash")
	}
	dir := t.TempDir()
	cmd := exec.Command(os.Args[0], "-test.run=^TestCrashOutputLandsInFile$")
	cmd.Env = append(os.Environ(), crashChildEnv+"="+dir, "GOTRACEBACK=")
	out, err := cmd.CombinedOutput()
	if err == nil {
		t.Fatalf("child did not crash:\n%s", out)
	}
	b, err := os.ReadFile(filepath.Join(dir, CrashName))
	if err != nil {
		t.Fatal(err)
	}
	s := string(b)
	for _, want := range []string{"role=test", "panic: diag test crash", "goroutine "} {
		if !strings.Contains(s, want) {
			t.Errorf("crash.log missing %q:\n%s", want, s)
		}
	}
}

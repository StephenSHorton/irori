// Package diag keeps the files that explain a crash after the fact: the app
// log (irori.log, rotated by size) and crash.log, where the Go runtime writes
// a fatal error or an unrecovered panic.
//
// The desktop app is a GUI-subsystem exe with no console, so without these a
// crash leaves nothing behind. v0.4.1 died twice that way; the second dump
// only exists because stderr happened to be captured.
package diag

import (
	"fmt"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"runtime/debug"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	// CrashName is the file in the data dir the runtime writes crashes to.
	CrashName = "crash.log"
	// LogName is the app log in the data dir.
	LogName = "irori.log"

	crashMax = 4 << 20 // crash.log moves to crash.log.1 past this at startup
	logMax   = 4 << 20 // irori.log rotates past this
	logKeep  = 3       // irori.log.1 .. irori.log.3
)

// InstallCrashOutput makes the runtime also write any fatal error or
// unrecovered panic to <dir>/crash.log, with every goroutine's stack. Every
// role calls it, so the file is opened for append and shared: a short-lived
// `irori mcp` that crashes lands in the same file as the app. The handle stays
// open for the life of the process, as SetCrashOutput needs.
func InstallCrashOutput(dir string) (*os.File, error) {
	debug.SetTraceback("all") // never lowers a GOTRACEBACK set in the environment
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, err
	}
	path := filepath.Join(dir, CrashName)
	if fi, err := os.Stat(path); err == nil && fi.Size() > crashMax {
		_ = os.Rename(path, path+".1") // fails if another role has it open; grow a little more
	}
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_APPEND, 0o600)
	if err != nil {
		return nil, err
	}
	if err := debug.SetCrashOutput(f, debug.CrashOptions{}); err != nil {
		_ = f.Close()
		return nil, err
	}
	// SetCrashOutput dups the handle; keeping f open too costs nothing and
	// lets the caller write a start line.
	return f, nil
}

// StartLine says which build is running, for the top of a log or before a
// crash: the toolchain matters (Go 1.26.0 and 1.26.1 corrupt goroutine
// stacks on Windows UDP receives).
func StartLine(role, version string) string {
	line := fmt.Sprintf("%s irori %s start: role=%s pid=%d %s %s/%s",
		time.Now().Format(time.RFC3339), version, role, os.Getpid(), runtime.Version(), runtime.GOOS, runtime.GOARCH)
	if bi, ok := debug.ReadBuildInfo(); ok {
		for _, s := range bi.Settings {
			if s.Key == "vcs.revision" || s.Key == "-tags" {
				line += " " + s.Key + "=" + s.Value
			}
		}
	}
	if BadToolchain(runtime.Version()) {
		line += " WARNING: this toolchain corrupts stacks on Windows UDP (golang/go#77975); rebuild with go1.26.2 or later"
	}
	return line + "\n"
}

// BadToolchain reports a Go release whose Windows WSARecvFrom left its
// address-length out-parameter on the goroutine stack, where the overlapped
// completion later writes 4 bytes into whatever that memory has become. That
// is what crashed v0.4.1 (built with go1.26.0); go1.26.2 moved it to the heap
// (CL 753480, "internal/poll: move rsan to heap on windows"). The upstream
// report also reproduced on go1.25.0 and no 1.25 backport exists, so every
// release before go1.26.2 counts. Development builds ("devel ...") do not.
func BadToolchain(v string) bool {
	rest, ok := strings.CutPrefix(v, "go1.")
	if !ok {
		return false
	}
	minorS, patchS, _ := strings.Cut(rest, ".")
	if i := strings.IndexAny(minorS, "rb"); i >= 0 { // go1.26rc1, go1.26beta1
		minorS, patchS = minorS[:i], "0"
	}
	minor, err := strconv.Atoi(minorS)
	if err != nil {
		return false
	}
	patch, _ := strconv.Atoi(patchS) // go1.21 and older have no ".0"
	return minor < 26 || (minor == 26 && patch < 2)
}

// Log is an append-only file that rotates by size: name, name.1 .. name.keep.
type Log struct {
	mu   sync.Mutex
	path string
	max  int64
	keep int
	f    *os.File
	size int64
}

// OpenLog opens <dir>/irori.log for the long-running roles (the app and
// serve --headless; only one of them runs per device).
func OpenLog(dir string) (*Log, error) {
	return openLog(filepath.Join(dir, LogName), logMax, logKeep)
}

func openLog(path string, max int64, keep int) (*Log, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return nil, err
	}
	l := &Log{path: path, max: max, keep: keep}
	if err := l.open(); err != nil {
		return nil, err
	}
	if l.size >= l.max {
		l.rotate()
	}
	return l, nil
}

func (l *Log) open() error {
	f, err := os.OpenFile(l.path, os.O_WRONLY|os.O_CREATE|os.O_APPEND, 0o600)
	if err != nil {
		return err
	}
	var size int64
	if fi, err := f.Stat(); err == nil {
		size = fi.Size()
	}
	l.f, l.size = f, size
	return nil
}

// rotate shifts name.N-1 to name.N and name to name.1, then starts a new
// name. A failed rename (another process holds the file) keeps writing to
// the current one rather than lose lines.
func (l *Log) rotate() {
	_ = l.f.Close()
	for i := l.keep; i > 1; i-- {
		_ = os.Rename(l.path+"."+strconv.Itoa(i-1), l.path+"."+strconv.Itoa(i))
	}
	_ = os.Remove(l.path + "." + strconv.Itoa(l.keep+1))
	_ = os.Rename(l.path, l.path+".1")
	if err := l.open(); err != nil {
		l.f, l.size = nil, 0
	}
}

// Write appends p, rotating first when p would take the file past max.
func (l *Log) Write(p []byte) (int, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.f == nil {
		if err := l.open(); err != nil {
			return 0, err
		}
	}
	if l.size > 0 && l.size+int64(len(p)) > l.max {
		l.rotate()
		if l.f == nil {
			return 0, os.ErrClosed
		}
	}
	n, err := l.f.Write(p)
	l.size += int64(n)
	return n, err
}

// Close closes the current file.
func (l *Log) Close() error {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.f == nil {
		return nil
	}
	err := l.f.Close()
	l.f = nil
	return err
}

var _ io.WriteCloser = (*Log)(nil)

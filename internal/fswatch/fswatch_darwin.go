//go:build darwin && cgo

package fswatch

/*
#cgo LDFLAGS: -framework CoreServices
#include <stdlib.h>
#include "fswatch_darwin.h"
*/
import "C"

import (
	"context"
	"errors"
	"path/filepath"
	"runtime/cgo"
	"strings"
	"sync"
	"sync/atomic"
	"time"
	"unsafe"
)

// FSEvents flags (CoreServices FSEvents.h).
const (
	flagMustScanSubDirs = 0x00000001
	flagUserDropped     = 0x00000002
	flagKernelDropped   = 0x00000004
	flagHistoryDone     = 0x00000010
	flagRootChanged     = 0x00000020
)

// latency is how long FSEvents may hold events before calling back. With
// NoDefer the first event of a quiet spell is delivered at once; coalesce
// does the real debouncing.
const latency = 0.01

// darwinStream is one FSEvents stream's Go side, reached from the C
// callback through a cgo.Handle.
type darwinStream struct {
	root     string // as the caller gave it (made absolute)
	realRoot string // root with symlinks resolved: what FSEvents reports
	recurse  bool
	raw      chan Event
	lost     *atomic.Bool
	closed   atomic.Bool
}

// watch uses one FSEvents stream for the tree. FSEvents always watches the
// whole subtree; a non-recursive watch drops events below root's direct
// children. Events dropped by the kernel or fseventsd, or a moved root,
// arrive as Overflow, the same contract as ReadDirectoryChangesW's.
func watch(ctx context.Context, root string, recursive bool, debounce time.Duration) (*Watcher, error) {
	root, err := filepath.Abs(root)
	if err != nil {
		return nil, err
	}
	real, err := filepath.EvalSymlinks(root)
	if err != nil {
		return nil, err
	}
	var lost atomic.Bool
	ds := &darwinStream{root: root, realRoot: real, recurse: recursive, raw: make(chan Event, 1024), lost: &lost}
	h := cgo.NewHandle(ds)
	croot := C.CString(real)
	defer C.free(unsafe.Pointer(croot))
	s := C.fswatchStart(C.uintptr_t(h), croot, C.double(latency))
	if s == nil {
		h.Delete()
		return nil, errors.New("fswatch: FSEventStreamCreate failed for " + root)
	}
	out := make(chan Event, 1024)
	go coalesce(ds.raw, out, debounce, root, &lost)

	var once sync.Once
	done := make(chan struct{})
	stop := func() {
		once.Do(func() {
			C.fswatchStop(s)
			ds.closed.Store(true)
			h.Delete()
			close(ds.raw)
			close(done)
		})
		<-done
	}
	go func() {
		select {
		case <-ctx.Done():
			stop()
		case <-done:
		}
	}()
	return &Watcher{events: out, stop: stop}, nil
}

//export goFSEvents
func goFSEvents(info C.uintptr_t, n C.size_t, paths **C.char, flags *C.uint32_t) {
	ds, ok := cgo.Handle(info).Value().(*darwinStream)
	if !ok || ds.closed.Load() {
		return
	}
	ps := unsafe.Slice(paths, int(n))
	fs := unsafe.Slice(flags, int(n))
	for i := range ps {
		ds.event(C.GoString(ps[i]), uint32(fs[i]))
	}
}

func (ds *darwinStream) event(p string, flags uint32) {
	if flags&(flagMustScanSubDirs|flagUserDropped|flagKernelDropped|flagRootChanged) != 0 {
		send(ds.raw, Event{Path: ds.root, Overflow: true}, ds.lost)
		return
	}
	if flags&flagHistoryDone != 0 {
		return
	}
	p, ok := ds.callerPath(p)
	if !ok {
		return
	}
	if !ds.recurse && p != ds.root && filepath.Dir(p) != ds.root {
		return
	}
	send(ds.raw, Event{Path: p}, ds.lost)
}

// callerPath maps a path FSEvents reports (symlinks resolved, so
// /private/var/... for /var/...) back under the root the caller asked for.
func (ds *darwinStream) callerPath(p string) (string, bool) {
	p = filepath.Clean(p)
	if p == ds.realRoot {
		return ds.root, true
	}
	rest, ok := strings.CutPrefix(p, ds.realRoot+string(filepath.Separator))
	if !ok {
		// FSEvents can report a case or Unicode-normalisation variant of
		// the root on APFS; match it case-insensitively before giving up.
		if len(p) > len(ds.realRoot) && strings.EqualFold(p[:len(ds.realRoot)], ds.realRoot) && p[len(ds.realRoot)] == filepath.Separator {
			rest, ok = p[len(ds.realRoot)+1:], true
		}
	}
	if !ok {
		return "", false
	}
	return filepath.Join(ds.root, rest), true
}

//go:build darwin && cgo

package awake

/*
#cgo LDFLAGS: -framework IOKit -framework CoreFoundation
#include <IOKit/pwr_mgt/IOPMLib.h>
#include <CoreFoundation/CoreFoundation.h>

// Idle system sleep only: the display may still sleep.
static IOReturn irori_awake_acquire(IOPMAssertionID *outID) {
	return IOPMAssertionCreateWithName(CFSTR("PreventUserIdleSystemSleep"), kIOPMAssertionLevelOn,
		CFSTR("Irori home server"), outID);
}

static IOReturn irori_awake_release(IOPMAssertionID id) {
	return IOPMAssertionRelease(id);
}
*/
import "C"

import (
	"fmt"
	"sync"
)

type darwinHold struct {
	once sync.Once
	id   C.IOPMAssertionID
}

func acquire() (Hold, error) {
	var id C.IOPMAssertionID
	if r := C.irori_awake_acquire(&id); r != 0 {
		return nil, fmt.Errorf("IOPMAssertionCreateWithName: 0x%x", uint32(r))
	}
	return &darwinHold{id: id}, nil
}

func (h *darwinHold) Release() {
	h.once.Do(func() { _ = C.irori_awake_release(h.id) })
}

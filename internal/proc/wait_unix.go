//go:build !windows

package proc

import (
	"context"
	"time"
)

// waitExit has no handle to wait on off Windows (a non-child pid can't be
// waited for portably); callers poll Alive.
func waitExit(context.Context, int, time.Time) error { return ErrCantWait }

package proc

import (
	"context"
	"errors"
	"time"
)

// ErrCantWait means WaitExit can't wait on pid here (another OS, or no
// handle to the process); callers poll Alive instead.
var ErrCantWait = errors.New("proc: can't wait on this process")

// WaitExit blocks until pid exits and returns nil, or returns ctx.Err() when
// ctx ends first. It also returns nil at once when pid is not running or the
// process now at pid started after startedBy (the pid was reused; zero skips
// that check). Once it holds the process it can't be fooled by reuse.
func WaitExit(ctx context.Context, pid int, startedBy time.Time) error {
	if pid <= 0 {
		return nil
	}
	return waitExit(ctx, pid, startedBy)
}

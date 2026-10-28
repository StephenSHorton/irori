package presence

import (
	"context"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/proc"
)

// watchExit calls onExit once pid's process ends (a kqueue NOTE_EXIT, via
// proc.WaitExit). stop ends the wait (without calling onExit) and waits for
// it. ok is false when the process is already gone; the safety rescan
// covers it, and covers a wait the kernel refuses (another user's process).
func watchExit(pid int, onExit func()) (stop func(), ok bool) {
	if !proc.Alive(pid) {
		return nil, false
	}
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() {
		defer close(done)
		if proc.WaitExit(ctx, pid, time.Time{}) == nil && ctx.Err() == nil {
			onExit()
		}
	}()
	var once sync.Once
	return func() {
		once.Do(func() {
			cancel()
			<-done
		})
	}, true
}

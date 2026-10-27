package main

import (
	"context"
	"sort"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/core"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// EventWorkChanged fires (no payload) when a task or lease changes, from
// whoever changed it, and when a lease runs out: coord's Subscribe feed,
// coalesced like inbox:changed. The Work tab re-reads on it.
const EventWorkChanged = "work:changed"

// workEmitEvery bounds work:changed to 4 events a second.
const workEmitEvery = 250 * time.Millisecond

func init() {
	application.RegisterEvent[struct{}](EventWorkChanged)
}

// WorkService is the person's view of tasks and leases. Actions run as the
// person at this device, who may cancel any task and release any lease.
// JS binding name: WorkService.*
type WorkService struct {
	core *core.Core
}

// Tasks lists tasks, newest change first; openOnly leaves out finished ones.
func (w *WorkService) Tasks(openOnly bool) []coord.Task {
	ts := nonNil(w.core.Coord().ListTasks(coord.TaskFilter{Open: openOnly}))
	sort.SliceStable(ts, func(i, j int) bool { return ts[i].UpdatedAt.After(ts[j].UpdatedAt) })
	return ts
}

// Leases lists unexpired leases, soonest expiry first.
func (w *WorkService) Leases() []coord.Lease {
	ls := nonNil(w.core.Coord().Leases())
	sort.SliceStable(ls, func(i, j int) bool { return ls[i].Until.Before(ls[j].Until) })
	return ls
}

// CancelTask moves a task to canceled; work:changed follows through pump.
func (w *WorkService) CancelTask(id string) (coord.Task, error) {
	return w.core.Coord().SetTaskState(context.Background(), w.core.UserSender().Key, id, coord.Canceled, "canceled from the Irori app")
}

// ReleaseLease drops the lease on resource, whoever holds it.
func (w *WorkService) ReleaseLease(resource string) error {
	return w.core.Coord().Release(context.Background(), w.core.UserSender().Key, resource)
}

// pump forwards coord changes to the webview as work:changed until ctx ends.
func (w *WorkService) pump(ctx context.Context, app *application.App) {
	pumpWork(ctx, w.core.Coord().Subscribe(ctx), workEmitEvery, func() {
		app.Event.Emit(EventWorkChanged, struct{}{})
	})
}

// pumpWork calls emit for changes, coalesced so a burst becomes one call at
// most every apart. It returns when ctx ends or changes closes.
func pumpWork(ctx context.Context, changes <-chan coord.Change, every time.Duration, emit func()) {
	var last time.Time
	timer := time.NewTimer(time.Hour)
	timer.Stop()
	defer timer.Stop()
	pending := false
	for {
		select {
		case <-ctx.Done():
			return
		case _, ok := <-changes:
			if !ok {
				return
			}
			if !pending {
				pending = true
				timer.Reset(max(0, every-time.Since(last)))
			}
		case <-timer.C:
			pending = false
			last = time.Now()
			emit()
		}
	}
}

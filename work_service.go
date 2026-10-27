package main

import (
	"context"
	"sort"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/core"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// EventWorkChanged fires (no payload) after the person cancels a task or
// releases a lease from the GUI. Coord has no change feed yet, so the Work
// tab also polls while it is open.
const EventWorkChanged = "work:changed"

func init() {
	application.RegisterEvent[struct{}](EventWorkChanged)
}

// WorkService is the person's view of tasks and leases. Actions run as the
// person at this device, who may cancel any task and release any lease.
// JS binding name: WorkService.*
type WorkService struct {
	core *core.Core
	app  *application.App // set by gui.go once the app exists; nil in tests
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

// CancelTask moves a task to canceled.
func (w *WorkService) CancelTask(id string) (coord.Task, error) {
	t, err := w.core.Coord().SetTaskState(context.Background(), w.core.UserSender().Key, id, coord.Canceled, "canceled from the Irori app")
	if err == nil {
		w.changed()
	}
	return t, err
}

// ReleaseLease drops the lease on resource, whoever holds it.
func (w *WorkService) ReleaseLease(resource string) error {
	err := w.core.Coord().Release(context.Background(), w.core.UserSender().Key, resource)
	if err == nil {
		w.changed()
	}
	return err
}

func (w *WorkService) changed() {
	if w.app != nil {
		w.app.Event.Emit(EventWorkChanged, struct{}{})
	}
}

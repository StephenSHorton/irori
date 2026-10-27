package coord

import (
	"context"
	"time"

	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// Skeleton stubs. The coord work package replaces this file.

type coord struct{}

func newCoord(context.Context, *store.Store, interface{ Agents() []presence.AgentCard }, Options) (*Coord, error) {
	return &Coord{impl: &coord{}}, nil
}
func (c *coord) createTask(context.Context, string, NewTask) (Task, error) {
	return Task{}, ErrUnimplemented
}
func (c *coord) listTasks(TaskFilter) []Task { return nil }
func (c *coord) getTask(string) (Task, bool) { return Task{}, false }
func (c *coord) claimTask(context.Context, string, string) (Task, error) {
	return Task{}, ErrUnimplemented
}
func (c *coord) assignTask(context.Context, string, string, string) (Task, error) {
	return Task{}, ErrUnimplemented
}
func (c *coord) setTaskState(context.Context, string, string, TaskState, string) (Task, error) {
	return Task{}, ErrUnimplemented
}
func (c *coord) acquire(context.Context, string, string, time.Duration, bool) (Lease, error) {
	return Lease{}, ErrUnimplemented
}
func (c *coord) release(context.Context, string, string) error { return ErrUnimplemented }
func (c *coord) leases() []Lease                               { return nil }
func (c *coord) sweep(context.Context) error                   { return nil }
func overlaps(string, string) bool                             { return false }

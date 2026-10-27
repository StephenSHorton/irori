package main

import (
	"context"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// EventRosterChanged carries a RosterChange whenever an agent appears, changes or leaves.
const EventRosterChanged = "roster:changed"

// RosterChange is the roster:changed payload: the change plus the full roster
// after it, so the UI never has to merge.
type RosterChange struct {
	Change presence.Change      `json:"change"`
	Agents []presence.AgentCard `json:"agents"`
}

// RosterService is the UI's view of who is running. JS binding name: RosterService.*
type RosterService struct {
	core *core.Core
}

// Agents returns the live agents on this device, newest activity first.
func (r *RosterService) Agents() []presence.AgentCard { return r.core.Agents() }

// Device returns this device's identity.
func (r *RosterService) Device() device.Identity { return r.core.Device() }

// Version returns the running Irori version ("dev" for local builds).
func (r *RosterService) Version() string { return r.core.Version() }

// Recent returns the newest log rows (newest first), at most limit (default 100).
func (r *RosterService) Recent(limit int) ([]store.Event, error) {
	if limit <= 0 {
		limit = 100
	}
	return r.core.Events(context.Background(), store.Filter{Limit: limit, Desc: true})
}

func (r *RosterService) pump(ctx context.Context, app *application.App) {
	for ch := range r.core.Changes(ctx) {
		app.Event.Emit(EventRosterChanged, RosterChange{Change: ch, Agents: r.core.Agents()})
	}
}

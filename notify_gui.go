package main

import (
	"context"
	"strings"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/notify"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// EventOpen asks the frontend to show a tab and, when Ref is set, select
// that thing there (a Board card "task:<id>", an Inbox delivery id). A
// notification's Open, Answer or body click sends it after showing the
// window.
const EventOpen = "app:open"

func init() {
	application.RegisterEvent[notify.Target](EventOpen)
}

// setupNotifications connects the Core's Notifier to the desktop app: its
// Open shows the window (show, the tray's and second launch's code) at the
// target's tab, and notify.Watch turns inbox and Board changes into cards
// until ctx ends. Call it once after the window exists.
func setupNotifications(ctx context.Context, app *application.App, c *core.Core, show func()) {
	n := c.Notifier()
	n.SetOpener(func(t notify.Target) {
		show()
		app.Event.Emit(EventOpen, t)
	})
	go notify.Watch(ctx, n, notify.Sources{
		Inbox:   c.Inbox(),
		Board:   c.Coord(),
		User:    c.UserSender().Key,
		IsOwner: c.IsOwner,
		Name:    func(key string) string { return agentName(c, key) },
	})
}

// agentName is an agent key's display name from the live roster, or "".
func agentName(c *core.Core, key string) string {
	for _, a := range c.Agents() {
		if a.Key == key {
			return strings.TrimSpace(a.Name)
		}
	}
	return ""
}

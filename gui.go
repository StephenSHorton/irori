package main

import (
	"context"
	"embed"
	"fmt"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/update"
	"github.com/wailsapp/wails/v3/pkg/application"
	"github.com/wailsapp/wails/v3/pkg/events"
)

//go:embed all:frontend/dist
var assets embed.FS

func init() {
	application.RegisterEvent[RosterChange](EventRosterChanged)
}

// runGUI runs the desktop app with an in-process Core. Irori lives in the
// tray: closing the window hides it, and Quit is in the tray menu. A
// --startup launch (launch at login) starts with the window hidden.
func runGUI(args []string) error {
	opts, err := coreOptions(false)
	if err != nil {
		return err
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	c, err := core.Start(ctx, opts)
	if err != nil {
		return fmt.Errorf("start: %w", err)
	}
	defer c.Close()

	roster := &RosterService{core: c}
	settings := &SettingsService{}
	upd := update.New(update.Repo, version)

	var window *application.WebviewWindow
	show := func() {
		if window == nil {
			return
		}
		window.Show()
		window.UnMinimise()
		window.Focus()
	}

	app := application.New(application.Options{
		Name:        "Irori",
		Description: "A hearth for your coding agents",
		Services: []application.Service{
			application.NewService(roster),
			application.NewService(settings),
			application.NewService(upd),
		},
		Assets: application.AssetOptions{
			Handler: application.AssetFileServerFS(assets),
		},
		Windows: application.WindowsOptions{
			// The tray keeps Irori running with no window open.
			DisableQuitOnLastWindowClosed: true,
		},
		Mac: application.MacOptions{
			ApplicationShouldTerminateAfterLastWindowClosed: false,
		},
		// A second launch (desktop icon while the login copy sits in the
		// tray) surfaces the running window instead of a second Core.
		SingleInstance: &application.SingleInstanceOptions{
			UniqueID:               "com.stephenshorton.irori",
			OnSecondInstanceLaunch: func(application.SecondInstanceData) { show() },
		},
	})
	settings.attach(app)

	window = app.Window.NewWithOptions(application.WebviewWindowOptions{
		Title:  "Irori",
		Width:  1100,
		Height: 720,
		Hidden: startedAtLogin(args),
		// Matches yardline's page background so the window doesn't flash.
		BackgroundColour: application.NewRGB(233, 238, 248),
		URL:              "/",
	})
	window.RegisterHook(events.Common.WindowClosing, func(e *application.WindowEvent) {
		window.Hide()
		e.Cancel()
	})

	tray := setupTray(app, show, version)
	setupUpdater(app, upd, tray)
	go roster.pump(ctx, app)
	go roster.trayStatus(ctx, tray)
	return app.Run()
}

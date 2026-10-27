package main

import (
	"context"
	"embed"
	"errors"
	"fmt"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/update"
	"github.com/wailsapp/wails/v3/pkg/application"
	"github.com/wailsapp/wails/v3/pkg/events"
)

//go:embed all:frontend/dist
var assets embed.FS

// singleInstanceID is shared by the app and handOff so a second launch reaches the first.
const singleInstanceID = "com.stephenshorton.irori"

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
	if errors.Is(err, core.ErrAlreadyRunning) {
		if lf, rerr := discovery.Read(opts.DataDir); rerr == nil && !lf.Headless {
			return handOff()
		}
	}
	if err != nil {
		return fmt.Errorf("start: %w", err)
	}
	defer c.Close()

	roster := &RosterService{core: c}
	inboxSvc := newInboxService(c)
	work := &WorkService{core: c}
	captureSvc := newCaptureService(c)
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
			application.NewService(inboxSvc),
			application.NewService(work),
			application.NewService(captureSvc),
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
			UniqueID:               singleInstanceID,
			OnSecondInstanceLaunch: func(application.SecondInstanceData) { show() },
		},
	})
	settings.attach(app)
	work.app = app

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
	tray.SetPause(inboxSvc.SetPaused)
	inboxSvc.onCounts(tray.SetInbox)
	go inboxSvc.pump(ctx, app)
	go captureSvc.pump(ctx, app)
	return app.Run()
}

// handOff runs when another desktop Irori already holds the Core: a bare app
// with the same single-instance id, so Wails passes the launch to the running
// copy (which shows its window) and exits this process.
func handOff() error {
	app := application.New(application.Options{
		Name: "Irori",
		SingleInstance: &application.SingleInstanceOptions{
			UniqueID: singleInstanceID,
		},
	})
	return app.Run()
}

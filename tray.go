// The tray lives in package main because its icon is //go:embed'ed, and embed
// paths can't climb out of the package directory with "..": build/appicon.png
// is only reachable from the repo root.
package main

import (
	_ "embed"
	"log"
	"sync"

	"github.com/StephenSHorton/irori/internal/update"
	"github.com/wailsapp/wails/v3/pkg/application"
	"github.com/wailsapp/wails/v3/pkg/events"
)

// trayIconPNG is the tray icon. Windows decodes PNG bytes and scales the
// 1024px image down to the small-icon size itself, so no .ico is needed.
//
//go:embed build/appicon.png
var trayIconPNG []byte

func init() {
	application.RegisterEvent[update.Status](update.EventUpdateStatus)
}

// TrayController owns the tray icon and its menu:
//
//	[Restart to update to vX]   only while an update is ready (SetUpdater)
//	Open Irori
//	<status line>               disabled; SetStatus changes it
//	Launch at login             checkbox, off until the user ticks it
//	Quit Irori
//
// The menu is rebuilt whole on every change, which is the path Wails handles
// reliably on Windows (the popup is recreated from the Menu). All methods are
// safe from any goroutine.
type TrayController struct {
	app     *application.App
	show    func()
	version string
	tray    *application.SystemTray

	mu      sync.Mutex
	status  string
	updater *update.UpdateService
}

// setupTray installs the tray icon and returns its controller. show opens or
// focuses the main window (tray left-click and "Open Irori"); version seeds
// the status line. Call it once, after application.New, before or after
// app.Run: Wails defers the native tray until the app is running.
func setupTray(app *application.App, show func(), version string) *TrayController {
	t := &TrayController{app: app, show: show, version: version}
	t.tray = app.SystemTray.New()
	t.tray.SetIcon(trayIconPNG)
	t.tray.SetTooltip("Irori")
	t.tray.OnClick(t.open)
	t.rebuild()
	// A toggle from the settings UI or a finished update changes the menu.
	app.Event.On(EventLaunchAtLogin, func(*application.CustomEvent) { t.rebuild() })
	app.Event.On(update.EventUpdateStatus, func(*application.CustomEvent) { t.rebuild() })
	return t
}

func versionLine(version string) string {
	if version == "" || version == "dev" {
		return "Irori (dev build)"
	}
	return "Irori v" + version
}

// SetStatus replaces the disabled status line, e.g. "3 agents running".
// An empty string restores the version line.
func (t *TrayController) SetStatus(s string) {
	if t == nil {
		return
	}
	t.mu.Lock()
	t.status = s
	t.mu.Unlock()
	t.rebuild()
}

// SetUpdater lets the menu offer "Restart to update" while an update is ready.
// The tray follows update:status events, so the service must be hooked up
// (setupUpdater) for the item to appear.
func (t *TrayController) SetUpdater(u *update.UpdateService) {
	if t == nil {
		return
	}
	t.mu.Lock()
	t.updater = u
	t.mu.Unlock()
	t.rebuild()
}

func (t *TrayController) open() {
	if t.show != nil {
		t.show()
	}
}

func (t *TrayController) rebuild() {
	t.mu.Lock()
	status, u := t.status, t.updater
	t.mu.Unlock()
	if status == "" {
		status = versionLine(t.version)
	}

	menu := application.NewMenu()
	if u != nil {
		if st := u.GetStatus(); st.Phase == update.PhaseReady {
			menu.Add("Restart to update to v" + st.Version).OnClick(func(*application.Context) {
				go func() {
					if err := u.RestartToUpdate(); err != nil {
						log.Printf("irori: tray restart to update: %v", err)
					}
				}()
			})
			menu.AddSeparator()
		}
	}

	menu.Add("Open Irori").OnClick(func(*application.Context) { t.open() })
	menu.Add(status).SetEnabled(false)
	menu.AddSeparator()

	on, err := launchAtLogin(t.app)
	if err != nil {
		log.Printf("irori: read launch at login: %v", err)
	}
	menu.AddCheckbox("Launch at login", on).OnClick(func(ctx *application.Context) {
		want := ctx.IsChecked()
		go func() {
			// setLaunchAtLogin announces the live state, which rebuilds the menu
			// (and snaps the checkbox back if the change failed).
			if err := setLaunchAtLogin(t.app, want); err != nil {
				log.Printf("irori: set launch at login: %v", err)
			}
		}()
	})
	menu.AddSeparator()
	menu.Add("Quit Irori").OnClick(func(*application.Context) { t.app.Quit() })

	t.tray.SetMenu(menu)
}

// setupUpdater connects the update service to the app: status events and
// quit, the background check once the app has started (including a silent
// --startup launch with no window), and install-on-quit. Call it once after
// application.New and before app.Run; u must also be in Options.Services for
// the UI to reach it. A nil tray is fine.
func setupUpdater(app *application.App, u *update.UpdateService, tray *TrayController) {
	u.SetHooks(func(name string, data any) { app.Event.Emit(name, data) }, app.Quit)
	tray.SetUpdater(u)
	app.Event.OnApplicationEvent(events.Common.ApplicationStarted, func(*application.ApplicationEvent) {
		go u.AutoUpdate() // dev builds and unsupported platforms return at once
	})
	// If an update is ready and the user quits, the installer runs silently
	// and leaves Irori closed (/NORELAUNCH).
	app.OnShutdown(u.InstallOnExit)
}

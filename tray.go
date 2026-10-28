// The tray lives in package main because its icon is //go:embed'ed, and embed
// paths can't climb out of the package directory with "..": build/appicon.png
// is only reachable from the repo root.
package main

import (
	_ "embed"
	"fmt"
	"log"
	"strings"
	"sync"

	"github.com/StephenSHorton/irori/internal/notify"
	"github.com/StephenSHorton/irori/internal/update"
	"github.com/wailsapp/wails/v3/pkg/application"
	"github.com/wailsapp/wails/v3/pkg/events"
)

// trayIconPNG is the tray icon on Windows and Linux: the 囲 logo on a tight
// white tile, so it stays legible at 16 px (the app icon's wider margin would
// shrink it). Windows decodes PNG bytes and scales it itself, so no .ico is
// needed. macOS uses a template image instead (tray_darwin.go).
//
//go:embed build/trayicon.png
var trayIconPNG []byte

func init() {
	application.RegisterEvent[update.Status](update.EventUpdateStatus)
}

// TrayController owns the tray icon and its menu:
//
//	[Restart to update to vX]   only while an update is ready (SetUpdater)
//	Open Irori
//	<status line>               disabled; SetStatus changes it, SetInbox adds unread/held,
//	                            SetDecide the cards waiting on a decision (every device)
//	Pause wakes on <device>     checkbox bound to this device's inbox policy (SetPause)
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

	mu       sync.Mutex
	status   string
	updater  *update.UpdateService
	counts   InboxCounts
	decide   int              // cards waiting on a decision, across devices
	device   string           // this device's name, for the pause checkbox
	setPause func(bool) error // nil until SetPause; hides the checkbox
}

// setupTray installs the tray icon and returns its controller. show opens or
// focuses the main window (tray left-click and "Open Irori"); version seeds
// the status line. Call it once, after application.New, before or after
// app.Run: Wails defers the native tray until the app is running.
func setupTray(app *application.App, show func(), version string) *TrayController {
	t := &TrayController{app: app, show: show, version: version}
	t.tray = app.SystemTray.New()
	setTrayIcon(t.tray)
	t.tray.SetTooltip("Irori")
	if trayClickOpensWindow {
		t.tray.OnClick(t.open)
	}
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

// SetInbox updates the unread/held suffix of the status line and the pause
// checkbox. Unchanged counts don't rebuild the menu.
func (t *TrayController) SetInbox(c InboxCounts) {
	if t == nil {
		return
	}
	t.mu.Lock()
	same := t.counts == c
	t.counts = c
	t.mu.Unlock()
	if !same {
		t.rebuild()
	}
}

// SetDecide sets the Needs-a-decision count on the status line. The board
// folds every device's rows, so it covers all of them (v0.4 §9).
// Unchanged counts don't rebuild the menu.
func (t *TrayController) SetDecide(n int) {
	if t == nil {
		return
	}
	t.mu.Lock()
	same := t.decide == n
	t.decide = n
	t.mu.Unlock()
	if !same {
		t.rebuild()
	}
}

// SetPause shows the "Pause wakes on <device>" checkbox; set turns
// pause-all on or off. Pause stays per device (decision D5): it holds
// wakes of this device's sessions, whoever sent the message.
func (t *TrayController) SetPause(device string, set func(bool) error) {
	if t == nil {
		return
	}
	t.mu.Lock()
	t.device, t.setPause = device, set
	t.mu.Unlock()
	t.rebuild()
}

// pauseLabel is the pause checkbox's text, naming the device it covers.
func pauseLabel(device string) string {
	if device == "" {
		return "Pause wakes on this device"
	}
	return "Pause wakes on " + device
}

// statusLine is status plus " · N need a decision · N unread · M held" for
// the nonzero counts.
func statusLine(status string, c InboxCounts, decide int) string {
	parts := []string{status}
	if decide > 0 {
		parts = append(parts, fmt.Sprintf("%d need a decision", decide))
	}
	if c.Unread > 0 {
		parts = append(parts, fmt.Sprintf("%d unread", c.Unread))
	}
	if c.Held > 0 {
		parts = append(parts, fmt.Sprintf("%d held", c.Held))
	}
	return strings.Join(parts, " · ")
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
	status, u, counts, decide, device, setPause := t.status, t.updater, t.counts, t.decide, t.device, t.setPause
	t.mu.Unlock()
	if status == "" {
		status = versionLine(t.version)
	}
	status = statusLine(status, counts, decide)

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

	if setPause != nil {
		menu.AddCheckbox(pauseLabel(device), counts.Paused).OnClick(func(ctx *application.Context) {
			want := ctx.IsChecked()
			go func() {
				// The policy change comes back through SetInbox, which
				// rebuilds the menu; on failure, rebuild to snap it back.
				if err := setPause(want); err != nil {
					log.Printf("irori: pause all wakes: %v", err)
					t.rebuild()
				}
			}()
		})
	}

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
//
// n is the GUI Core's Notifier: once an update is downloaded and verified it
// posts the update card (update_card.go), and show is the card's body click.
// A nil n posts nothing.
func setupUpdater(app *application.App, u *update.UpdateService, tray *TrayController, n *notify.Notifier, show func()) {
	u.SetHooks(func(name string, data any) { app.Event.Emit(name, data) }, app.Quit)
	tray.SetUpdater(u)
	if n != nil {
		card := &updateCard{n: n, install: u.RestartToUpdate, open: show}
		u.SetOnChange(card.onStatus)
	}
	app.Event.OnApplicationEvent(events.Common.ApplicationStarted, func(*application.ApplicationEvent) {
		go u.AutoUpdate() // dev builds and unsupported platforms return at once
	})
	// If an update is ready and the user quits, the installer runs silently
	// and leaves Irori closed (/NORELAUNCH).
	app.OnShutdown(u.InstallOnExit)
}

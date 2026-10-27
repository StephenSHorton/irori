package main

import (
	"errors"
	"slices"
	"sync"

	"github.com/wailsapp/wails/v3/pkg/application"
)

// startupArg is the flag Irori passes to itself when it launches at login
// (see SettingsService.SetLaunchAtLogin). A login launch starts quietly in
// the tray without opening the window; startedAtLogin detects it.
const startupArg = "--startup"

// autostartID is the registration name: the value under
// HKCU\Software\Microsoft\Windows\CurrentVersion\Run on Windows, the
// LaunchAgent label or SMAppService key on macOS. Pinning it keeps one stable
// entry even if the display name changes.
const autostartID = "Irori"

// EventLaunchAtLogin carries the new launch-at-login state (bool) whenever it
// changes, from the settings UI or the tray, so both stay in step.
const EventLaunchAtLogin = "settings:launch-at-login"

func init() {
	application.RegisterEvent[bool](EventLaunchAtLogin)
}

// startedAtLogin reports whether args (os.Args[1:]) mark a launch-at-login
// start, in which case the integrator should not open the window.
func startedAtLogin(args []string) bool { return slices.Contains(args, startupArg) }

// SettingsService holds app-level preferences. Launch at login is backed by
// Wails' AutostartManager: the OS registration is the source of truth, so
// nothing is persisted beside it. It is OFF by default and only ever turned on
// by the user (SetLaunchAtLogin(true) from the UI or the tray checkbox).
//
// JS binding name: SettingsService.*
type SettingsService struct {
	mu  sync.Mutex
	app *application.App
}

// attach injects the app after application.New: the App does not exist yet
// when the service list is built.
func (s *SettingsService) attach(app *application.App) {
	s.mu.Lock()
	s.app = app
	s.mu.Unlock()
}

func (s *SettingsService) getApp() (*application.App, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.app == nil {
		return nil, errors.New("settings: app not ready")
	}
	return s.app, nil
}

// GetLaunchAtLogin reports whether Irori is registered to start at login. It
// reads the live OS state, so it reflects changes made outside the app.
func (s *SettingsService) GetLaunchAtLogin() (bool, error) {
	app, err := s.getApp()
	if err != nil {
		return false, err
	}
	return launchAtLogin(app)
}

// SetLaunchAtLogin registers or removes the login launch. Enabling registers
// Irori with --startup so the login launch stays in the tray. Re-enabling is
// safe: the registration is overwritten.
func (s *SettingsService) SetLaunchAtLogin(enabled bool) error {
	app, err := s.getApp()
	if err != nil {
		return err
	}
	return setLaunchAtLogin(app, enabled)
}

// launchAtLogin reads the registration; shared by the service and the tray.
func launchAtLogin(app *application.App) (bool, error) { return app.Autostart.IsEnabled() }

// setLaunchAtLogin changes the registration and announces the result on
// EventLaunchAtLogin; shared by the service and the tray.
func setLaunchAtLogin(app *application.App, enabled bool) error {
	var err error
	if enabled {
		err = app.Autostart.EnableWithOptions(application.AutostartOptions{
			Identifier: autostartID,
			Arguments:  []string{startupArg},
		})
	} else {
		err = app.Autostart.Disable()
	}
	// Announce the live state either way, so a failed toggle snaps back.
	if now, rerr := launchAtLogin(app); rerr == nil {
		app.Event.Emit(EventLaunchAtLogin, now)
	}
	return err
}

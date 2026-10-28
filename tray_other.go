//go:build !darwin

package main

import "github.com/wailsapp/wails/v3/pkg/application"

func setTrayIcon(t *application.SystemTray) { t.SetIcon(trayIconPNG) }

// trayClickOpensWindow: on Windows a left click on the tray icon opens the
// window and a right click the menu.
const trayClickOpensWindow = true

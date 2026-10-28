package main

import (
	_ "embed"

	"github.com/wailsapp/wails/v3/pkg/application"
)

// trayTemplatePNG is the macOS menu bar icon: the mono 囲 mark
// (assets/brand/irori-mark-mono.svg) in black on transparency, 44 px for a
// Retina menu bar. As a template image macOS recolours it for light and dark
// menu bars and for the highlighted state, the way system items look.
//
//go:embed build/trayicon-template.png
var trayTemplatePNG []byte

func setTrayIcon(t *application.SystemTray) { t.SetTemplateIcon(trayTemplatePNG) }

// trayClickOpensWindow is false on macOS: a click on a menu bar item opens
// its menu there (Open Irori is its first item), as every other one does.
const trayClickOpensWindow = false

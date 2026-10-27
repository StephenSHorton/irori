package main

import (
	"context"
	"embed"
	"fmt"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/wailsapp/wails/v3/pkg/application"
)

//go:embed all:frontend/dist
var assets embed.FS

func init() {
	application.RegisterEvent[RosterChange](EventRosterChanged)
}

// runGUI runs the desktop app with an in-process Core.
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
	app := application.New(application.Options{
		Name:        "Irori",
		Description: "A hearth for your coding agents",
		Services: []application.Service{
			application.NewService(roster),
		},
		Assets: application.AssetOptions{
			Handler: application.AssetFileServerFS(assets),
		},
		SingleInstance: &application.SingleInstanceOptions{
			UniqueID: "com.stephenshorton.irori",
		},
	})
	app.Window.NewWithOptions(application.WebviewWindowOptions{
		Title:            "Irori",
		Width:            1100,
		Height:           720,
		BackgroundColour: application.NewRGB(245, 243, 239),
		URL:              "/",
	})
	go roster.pump(ctx, app)
	return app.Run()
}

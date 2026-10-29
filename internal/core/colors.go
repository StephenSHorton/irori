package core

import (
	"context"
	"errors"
	"fmt"

	"github.com/StephenSHorton/irori/internal/devcolor"
	"github.com/StephenSHorton/irori/internal/store"
)

// followColors keeps c.colors current: every device.color row, and a
// reload after a resync (card #22).
func (c *Core) followColors(ctx context.Context) {
	sub := c.store.Subscribe(ctx)
	c.loadColors(ctx)
	for ev := range sub {
		switch {
		case ev.Seq == 0:
			c.loadColors(ctx)
		case ev.Kind == devcolor.Kind:
			c.colors.Apply(ev)
		}
	}
}

func (c *Core) loadColors(ctx context.Context) {
	f := store.Filter{Kinds: []string{devcolor.Kind}, Limit: store.MaxListLimit}
	for {
		evs, err := c.store.List(ctx, f)
		if err != nil {
			return
		}
		for _, ev := range evs {
			c.colors.Apply(ev)
		}
		if len(evs) < f.Limit {
			return
		}
		f.After = evs[len(evs)-1].Cursor()
	}
}

// DeviceColors is each of the person's devices' colour, by device id: the
// person's pick, else a default no other device has (devcolor.Assign).
// Removed devices are left out.
func (c *Core) DeviceColors() map[string]string {
	var ds []devcolor.Device
	for _, d := range c.chain.Devices() {
		if !d.Revoked {
			ds = append(ds, devcolor.Device{ID: d.Cert.DeviceID, At: d.Cert.At})
		}
	}
	if len(ds) == 0 {
		ds = []devcolor.Device{{ID: c.device.ID}}
	}
	return devcolor.Assign(ds, c.colors.Chosen())
}

// SetDeviceColor records the person's colour pick for a device ("" goes
// back to the default). Only the GUI calls it (a user row, v0.4 §2).
func (c *Core) SetDeviceColor(ctx context.Context, deviceID, color string) error {
	if color != "" && !devcolor.Valid(color) {
		return fmt.Errorf("%q isn't one of the device colours", color)
	}
	if _, ok := c.DeviceColors()[deviceID]; !ok {
		return errors.New("that isn't one of your devices")
	}
	_, err := c.store.Append(ctx, "user", devcolor.Kind, map[string]any{"deviceId": deviceID, "color": color, "at_device": c.device.ID})
	return err
}

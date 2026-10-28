package main

import (
	"context"
	"errors"
	"fmt"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/pair"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// EventPairingChanged carries pair.Status whenever a pairing moves on, so
// the Devices dialog follows it without polling.
const EventPairingChanged = "devices:pairing"

func init() {
	application.RegisterEvent[pair.Status](EventPairingChanged)
}

// DevicesService is Settings → Devices (v0.4 §3, §9): the person's devices
// with one status dot each, pairing (Add a device, Join with a code, the
// words) and Remove. Pairing and Remove are Wails bindings only, never on
// the loopback API, so an agent with a shell can't pair or remove a device.
//
// JS binding name: DevicesService.*
type DevicesService struct {
	core *core.Core
}

func newDevicesService(c *core.Core) *DevicesService { return &DevicesService{core: c} }

// attach forwards pairing changes to the window.
func (s *DevicesService) attach(app *application.App) {
	s.core.Pairing().SetOnChange(func(st pair.Status) { app.Event.Emit(EventPairingChanged, st) })
}

// DeviceView is one device in the dialog.
type DeviceView struct {
	DeviceID string `json:"deviceId"`
	Name     string `json:"name"`
	Role     string `json:"role"`
	This     bool   `json:"this,omitempty"`
	// State is connected, recent or offline (this device: "this").
	State   string `json:"state"`
	Revoked bool   `json:"revoked,omitempty"`
	// Warning is the one line shown under a device when something is wrong.
	Warning string `json:"warning,omitempty"`
}

// Devices lists the person's devices, this one first.
func (s *DevicesService) Devices() []DeviceView {
	self := s.core.Device()
	net := s.core.NetStatus()
	out := []DeviceView{{DeviceID: self.ID, Name: self.Name, Role: identity.RoleDesktop, This: true, State: "this"}}
	if net.State == "removed" {
		out[0].Warning = "Removed from your devices. Reset this device to pair it again."
	} else if net.Restoring != "" {
		out[0].Warning = "Restoring this device's rows from another device."
	}
	for _, d := range s.core.CertifiedDevices() {
		if d.Cert.DeviceID == self.ID {
			out[0].Role = d.Cert.Role
		}
	}
	for _, p := range net.Peers {
		v := DeviceView{DeviceID: p.DeviceID, Name: p.Label, Role: p.Role, State: p.State, Revoked: p.Revoked}
		switch {
		case p.Revoked:
			v.Warning = "Removed."
		case p.Stopped != "":
			v.Warning = "It " + p.Stopped + ". Remove it."
		case p.Old:
			v.Warning = "It runs an older Irori. Update it."
		case p.Ahead:
			v.Warning = "Its clock is ahead: its rows are waiting."
		case p.Refused > 0:
			v.Warning = fmt.Sprintf("%d of its rows were refused.", p.Refused)
		}
		out = append(out, v)
	}
	return out
}

// AddDevice opens a pairing window (§3 step 1): the code, the QR text, and
// the role the new device gets ("desktop" or "node").
func (s *DevicesService) AddDevice(role string) (pair.Status, error) {
	return s.core.Pairing().Add(role)
}

// JoinWithCode enters a code from another device (§3 step 2).
func (s *DevicesService) JoinWithCode(code string) (pair.Status, error) {
	return s.core.Pairing().Join(code)
}

// ConfirmWords takes the words the new device shows, typed on this one.
// It returns an error for wrong words; the second miss closes the window.
func (s *DevicesService) ConfirmWords(words string) error {
	err := s.core.Pairing().Confirm(context.Background(), words)
	if errors.Is(err, pair.ErrWrongWords) {
		return errors.New("those aren't the words the other device shows")
	}
	return err
}

// PairingStatus is the open (or last) pairing.
func (s *DevicesService) PairingStatus() pair.Status { return s.core.Pairing().Status() }

// CancelPairing closes the open pairing, either side.
func (s *DevicesService) CancelPairing() { s.core.Pairing().Cancel() }

// RemoveDevice revokes a device (§3, Revoke): its later rows are refused
// everywhere and the person key rotates.
func (s *DevicesService) RemoveDevice(deviceID string) error {
	return s.core.Mesh().Revoke(context.Background(), deviceID, "removed in Settings")
}

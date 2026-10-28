package main

import (
	"context"
	"errors"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/mesh"
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
	app  *application.App
}

func newDevicesService(c *core.Core) *DevicesService { return &DevicesService{core: c} }

// attach forwards pairing changes to the window.
func (s *DevicesService) attach(app *application.App) {
	s.app = app
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
	// Action is what the warning asks for here: "reset" or "remove".
	Action string `json:"action,omitempty"`
	// LastSeen is when this device last heard from it (zero: never, or
	// this device).
	LastSeen time.Time `json:"lastSeen,omitzero"`
}

// Devices lists the person's devices, this one first (the same rows as
// GET /v1/devices).
func (s *DevicesService) Devices() []DeviceView {
	rows := s.core.Mesh().Devices(false)
	out := make([]DeviceView, 0, len(rows))
	for _, r := range rows {
		out = append(out, DeviceView{DeviceID: r.DeviceID, Name: r.Name, Role: r.Role, This: r.This,
			State: r.State, Revoked: r.Revoked, Warning: r.Warning, Action: r.Action, LastSeen: r.LastSeen})
	}
	return out
}

// AddDevice opens a pairing window (§3 step 1): the code and
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

// ConfirmKey takes, on the joining device, the key the other device shows
// beside its code. The joining device takes the pairing only from the
// device whose connection proved that key; the second miss closes it.
func (s *DevicesService) ConfirmKey(key string) error {
	err := s.core.Pairing().ConfirmKey(context.Background(), key)
	if errors.Is(err, pair.ErrWrongKey) {
		return errors.New("that isn't the key the other device shows")
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

// ResetDevice makes this device a new one (§3, Reset): its log, device id
// and keys move aside (nothing is deleted) and only the settings stay. It
// is the way back for a device that was removed or signed two histories.
// The Reset runs at the next start, so Irori quits now; opening it again
// finishes it.
func (s *DevicesService) ResetDevice() error {
	if err := s.core.RequestReset(); err != nil {
		return err
	}
	if s.app != nil {
		go func() {
			time.Sleep(300 * time.Millisecond) // the call returns to the window first
			s.app.Quit()
		}()
	}
	return nil
}

// RelayView is the Relay line in Devices (§4.4, §9): "Direct only" when no
// TURN relay is set; with one, its credential URL and how it is doing.
type RelayView struct {
	CredentialURL string `json:"credentialUrl,omitempty"`
	// State is none (Direct only), pending, ok or error.
	State  string `json:"state"`
	Detail string `json:"detail,omitempty"`
}

// Relay is the relay setting and its status.
func (s *DevicesService) Relay() RelayView {
	r, st := s.core.Relay()
	// The URL's query (often the provider's API key) never reaches the window.
	return RelayView{CredentialURL: mesh.ScrubURLs(r.CredentialURL), State: st.State, Detail: mesh.ScrubURLs(st.Detail)}
}

// SetRelay sets the TURN REST credential URL ("" is Direct only). It
// applies to connections made from now on.
func (s *DevicesService) SetRelay(credentialURL string) (RelayView, error) {
	if err := s.core.SetRelay(mesh.Relay{CredentialURL: strings.TrimSpace(credentialURL)}); err != nil {
		return RelayView{}, err
	}
	return s.Relay(), nil
}

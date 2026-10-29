// Package awake keeps this PC from idling into sleep while Irori asks it
// to: the home server holds it for as long as it serves. Only system sleep
// is held off: the display may still turn off, and a lid close or the
// Start menu's Sleep still sleeps the PC.
//
// Ported from suzuri's internal/caffeine (MIT, Stephen S. Horton): Windows
// SetThreadExecutionState with ES_SYSTEM_REQUIRED (no display flag), macOS
// an IOKit PreventUserIdleSystemSleep assertion. Elsewhere it does nothing.
package awake

// Holder acquires holds.
type Holder interface {
	// Acquire holds off idle system sleep until the Hold is released.
	Acquire() (Hold, error)
}

// Hold is one acquired hold.
type Hold interface {
	// Release lets the PC sleep again; a second call does nothing.
	Release()
}

// Reason is what macOS shows for the assertion (pmset -g assertions).
const Reason = "Irori home server"

// System is this OS's holder.
func System() Holder { return system{} }

type system struct{}

func (system) Acquire() (Hold, error) { return acquire() }

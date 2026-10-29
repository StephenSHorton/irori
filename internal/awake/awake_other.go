//go:build !windows && !(darwin && cgo)

package awake

// No power API here (or a macOS build without cgo): holding does nothing.
type noHold struct{}

func (noHold) Release() {}

func acquire() (Hold, error) { return noHold{}, nil }

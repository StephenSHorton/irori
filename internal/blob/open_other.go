//go:build !windows

package blob

// transientOpen is false off Windows: a rename never blocks an open there.
func transientOpen(err error) bool { return false }

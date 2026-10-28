//go:build !windows && !darwin

package presence

// watchExit has no backend off Windows yet; exits are seen by the rescan.
func watchExit(int, func()) (func(), bool) { return nil, false }

//go:build windows

package update

import "os/exec"

// supported: Windows updates by running the release's NSIS installer.
const supported = true

var errUnsupported error

func newPlatform(s *UpdateService) platform { return nsisPlatform(s) }

// detach: a child of a GUI process already outlives it on Windows.
func detach(*exec.Cmd) {}

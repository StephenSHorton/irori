//go:build !windows && !darwin

package update

import (
	"errors"
	"os/exec"
	"runtime"
)

// supported: only Windows and macOS update themselves so far.
const supported = false

var errUnsupported = errors.New("updates not supported on " + runtime.GOOS + " yet")

func newPlatform(*UpdateService) platform { return platform{assetKind: runtime.GOOS} }

func detach(*exec.Cmd) {}

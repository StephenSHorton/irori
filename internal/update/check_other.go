//go:build !windows

package update

import (
	"errors"
	"runtime"
)

// supported: only Windows updates itself so far. The macOS path (download the
// .app.zip, swap the bundle, re-sign; see suzuri's internal/update) is owed.
const supported = false

var errUnsupported = func() error {
	if runtime.GOOS == "darwin" {
		return errors.New("updates not supported on macOS yet")
	}
	return errors.New("updates not supported on " + runtime.GOOS + " yet")
}()

// CheckForUpdate always fails here: this platform cannot update itself yet.
func (s *UpdateService) CheckForUpdate() (*UpdateInfo, error) { return nil, errUnsupported }

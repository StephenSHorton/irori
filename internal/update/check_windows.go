//go:build windows

package update

// supported: Windows updates by running the release's NSIS installer.
const supported = true

var errUnsupported error

// CheckForUpdate returns the available update, or nil if Irori is up to date
// (or a "dev" build, which never reports updates).
func (s *UpdateService) CheckForUpdate() (*UpdateInfo, error) { return s.findUpdate() }

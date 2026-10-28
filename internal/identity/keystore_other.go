//go:build !windows && !darwin

package identity

// DefaultKeystore off Windows and macOS is the 0600 file.
func DefaultKeystore(dataDir string) Keystore { return FileKeystore{} }

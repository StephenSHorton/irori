//go:build !windows && !darwin

package blob

// MarkDownloaded is a no-op where the OS has no download mark.
func MarkDownloaded(path string) error { return nil }

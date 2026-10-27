//go:build !windows

package core

// procStartField: presence ignores procStart off Windows.
func procStartField() string { return "" }

//go:build !windows

package main

// useParentConsole is a Windows concern: elsewhere the exe always inherits
// the terminal's streams.
func useParentConsole(string) {}

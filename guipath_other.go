//go:build !darwin

package main

// fixGUIPath is a macOS concern (guipath_darwin.go): elsewhere a desktop
// app gets the same PATH as the user's shells.
func fixGUIPath() {}

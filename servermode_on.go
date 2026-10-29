//go:build server

package main

// wailsServerMode: built with Wails' server tag, which would serve the
// window's bindings and the attachment route over plain HTTP on
// localhost:8080 with no sign-in. runGUI refuses to start.
const wailsServerMode = true

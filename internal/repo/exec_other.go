//go:build !windows

package repo

import "os/exec"

func hideWindow(*exec.Cmd) {}

// sameEnvName compares environment variable names, case-sensitively.
func sameEnvName(a, b string) bool { return a == b }

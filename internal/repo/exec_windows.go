package repo

import (
	"os/exec"
	"strings"
	"syscall"
)

// createNoWindow is CREATE_NO_WINDOW from the Win32 process creation flags.
const createNoWindow = 0x08000000

func hideWindow(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true, CreationFlags: createNoWindow}
}

// sameEnvName compares environment variable names; Windows ignores case.
func sameEnvName(a, b string) bool { return strings.EqualFold(a, b) }

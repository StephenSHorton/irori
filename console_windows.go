package main

import (
	"log"
	"os"

	"golang.org/x/sys/windows"
)

const attachParentProcess = ^uint32(0) // ATTACH_PARENT_PROCESS

var procAttachConsole = windows.NewLazySystemDLL("kernel32.dll").NewProc("AttachConsole")

// useParentConsole gives a command-line role the console it was typed in,
// for whichever of stdout and stderr the GUI-subsystem exe started without.
func useParentConsole(cmd string) {
	if !wantsConsole(cmd) {
		return
	}
	out, errf := fixStdio(
		stdUsable(windows.STD_OUTPUT_HANDLE),
		stdUsable(windows.STD_ERROR_HANDLE),
		func() bool {
			r, _, _ := procAttachConsole.Call(uintptr(attachParentProcess))
			return r != 0
		},
		func() (*os.File, error) { return os.OpenFile("CONOUT$", os.O_WRONLY, 0) },
	)
	if out != nil {
		os.Stdout = out
	}
	if errf != nil {
		os.Stderr = errf
		log.SetOutput(errf) // the log package captured the old os.Stderr at init
	}
}

// stdUsable reports whether the inherited standard handle leads to a console,
// a pipe or a file. A GUI-subsystem process started from a prompt gets none.
func stdUsable(which uint32) bool {
	h, err := windows.GetStdHandle(which)
	if err != nil || h == 0 || h == windows.InvalidHandle {
		return false
	}
	t, err := windows.GetFileType(h)
	return err == nil && t != windows.FILE_TYPE_UNKNOWN
}

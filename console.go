package main

import "os"

// The release exe is linked with -H windowsgui so the desktop app opens no
// console window. The cost is that a GUI-subsystem process typed at a prompt
// gets no stdout or stderr at all: `irori version` would print nothing. The
// command-line roles below attach to the parent's console instead, but only
// for the streams that have nowhere to go, so a pipe or a file the caller set
// up (the mod's `irori attach`, `irori version | more`) is left alone.

// wantsConsole reports whether cmd is a command-line role that prints for a
// person at a prompt. The GUI never wants one, and `irori mcp` speaks only on
// the stdio pipes its agent gave it.
func wantsConsole(cmd string) bool {
	switch cmd {
	case "", "gui", "mcp":
		return false
	}
	return true // version, serve, attach, events, and unknown commands (their error)
}

// fixStdio decides the replacement stdout and stderr. outOK and errOK say
// whether the inherited handles already lead somewhere. attach joins the
// parent's console and reports success; open opens that console for writing.
// A nil result means keep the inherited stream.
func fixStdio(outOK, errOK bool, attach func() bool, open func() (*os.File, error)) (out, errf *os.File) {
	if outOK && errOK {
		return nil, nil
	}
	if !attach() {
		return nil, nil // started without a console (Explorer, a service): nothing to join
	}
	f, err := open()
	if err != nil {
		return nil, nil
	}
	if !outOK {
		out = f
	}
	if !errOK {
		errf = f
	}
	return out, errf
}

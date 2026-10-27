package main

import (
	"errors"
	"os"
	"testing"
)

func TestWantsConsole(t *testing.T) {
	for cmd, want := range map[string]bool{
		"": false, "gui": false, "mcp": false,
		"version": true, "serve": true, "attach": true, "events": true, "bogus": true,
	} {
		if got := wantsConsole(cmd); got != want {
			t.Errorf("wantsConsole(%q) = %v, want %v", cmd, got, want)
		}
	}
}

func TestFixStdio(t *testing.T) {
	con := os.NewFile(0, "CONOUT$") // stand-in; never written
	open := func() (*os.File, error) { return con, nil }
	attached := 0
	attach := func() bool { attached++; return true }

	// Typed at a prompt: neither stream exists, both go to the console.
	out, errf := fixStdio(false, false, attach, open)
	if out != con || errf != con {
		t.Fatalf("no handles: got out=%v err=%v, want both console", out, errf)
	}

	// `irori version | more`: stdout is a pipe and stays one; stderr gets the console.
	out, errf = fixStdio(true, false, attach, open)
	if out != nil || errf != con {
		t.Fatalf("piped stdout: got out=%v err=%v", out, errf)
	}

	// The mod's `irori attach` or any fully redirected run: leave it all, never attach.
	attached = 0
	out, errf = fixStdio(true, true, attach, open)
	if out != nil || errf != nil || attached != 0 {
		t.Fatalf("redirected: got out=%v err=%v attached=%d", out, errf, attached)
	}

	// Started with no console to join (Explorer, a scheduled task): keep what there is.
	out, errf = fixStdio(false, false, func() bool { return false }, open)
	if out != nil || errf != nil {
		t.Fatalf("no parent console: got out=%v err=%v", out, errf)
	}
	out, errf = fixStdio(false, false, attach, func() (*os.File, error) { return nil, errors.New("no") })
	if out != nil || errf != nil {
		t.Fatalf("open failed: got out=%v err=%v", out, errf)
	}
}

package main

import (
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/mesh"
)

// The Remote line names an unreachable home server by its device name,
// and says nothing while every server is reachable or still connecting.
func TestRemoteLine(t *testing.T) {
	devices := []mesh.DeviceRow{{DeviceID: "pc1", Name: "Desk PC"}}
	if got := remoteLine([]core.RemoteServer{{DeviceID: "pc1", Up: false}}, devices); got != "" {
		t.Fatalf("a hub still connecting got a line: %q", got)
	}
	got := remoteLine([]core.RemoteServer{{DeviceID: "pc1", Unreachable: true}}, devices)
	if !strings.Contains(got, "Can't reach your home server (Desk PC)") {
		t.Fatalf("line %q", got)
	}
	if got := remoteLine([]core.RemoteServer{{DeviceID: "x9", Unreachable: true}}, nil); !strings.Contains(got, "(x9)") {
		t.Fatalf("an unnamed server: %q", got)
	}
}

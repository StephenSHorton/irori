package core

import (
	"context"
	"runtime"
	"testing"

	"github.com/StephenSHorton/irori/internal/api"
)

// GET /v1/devices lists the person's devices, this one first, with the
// state Devices shows; detail=1 adds what is behind it; health says the
// net is on (v0.4 §6.8). Revoked devices stay listed, marked.
func TestDevicesRouteAndHealthNet(t *testing.T) {
	ctx := context.Background()
	_, pc, mac, nd := meshTrio(t)
	c, err := api.Dial(ctx, pc.data)
	if err != nil {
		t.Fatal(err)
	}
	wait(t, "both peers connected", func() bool {
		rows, err := c.Devices(ctx, false)
		if err != nil {
			t.Fatal(err)
		}
		if len(rows) != 3 || !rows[0].This || rows[0].DeviceID != pc.id() || rows[0].State != "this" {
			t.Fatalf("rows = %+v", rows)
		}
		up := 0
		for _, r := range rows[1:] {
			if r.Detail != nil {
				t.Fatal("detail without detail=1")
			}
			if r.State == "connected" && r.OS == runtime.GOOS {
				up++
			}
		}
		return up == 2
	})
	rows, err := c.Devices(ctx, true)
	if err != nil {
		t.Fatal(err)
	}
	for _, r := range rows[1:] {
		if r.Detail == nil || r.Detail.Path != "direct" {
			t.Fatalf("detail = %+v", r)
		}
		if r.DeviceID == nd.id() && r.Role != "node" {
			t.Fatalf("the node's role = %q", r.Role)
		}
	}
	h, err := c.Health(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if h.Net == nil || h.Net.State != "on" || h.Net.Peers != 2 {
		t.Fatalf("health net = %+v", h.Net)
	}

	if err := pc.c.Mesh().Revoke(ctx, mac.id(), "test"); err != nil {
		t.Fatal(err)
	}
	rows, _ = c.Devices(ctx, false)
	for _, r := range rows {
		if r.DeviceID == mac.id() && (!r.Revoked || r.Warning == "") {
			t.Fatalf("the removed mac = %+v", r)
		}
	}
}

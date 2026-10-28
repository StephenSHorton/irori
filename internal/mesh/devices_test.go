package mesh

import (
	"context"
	"errors"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/StephenSHorton/irori/internal/store"
)

// downNet is a transport whose signaling never comes up: no broker is
// reachable.
type downNet struct{ joins atomic.Int32 }

func (d *downNet) Join(Secret, RoomConfig) (Room, error) {
	d.joins.Add(1)
	return nil, errors.New("no broker reachable")
}

// Single device, §10: with the brokers unreachable the device keeps
// working on its own, Devices says why, and the join is retried.
func TestUnreachableSignalingLeavesAWorkingDevice(t *testing.T) {
	a, b := newDev(t, NewFakeNet(), "pc"), newDev(t, NewFakeNet(), "mac")
	b.join(a) // a has a second device, so its net starts
	down := &downNet{}
	a.node = New(Options{Store: a.st, Ident: a.ident, Chain: a.chain, Net: down, Version: "0.4.0-test",
		Files: a.files, Intervals: fastIntervals, Now: a.clk.Now, Logf: t.Logf})
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() { defer close(done); _ = a.node.Run(ctx) }()
	defer func() { cancel(); <-done }()

	eventually(t, "the join is retried", func() bool { return down.joins.Load() >= 3 })
	st := a.node.Status()
	if st.State != StateError || !strings.Contains(st.Detail, "Can't reach the signaling servers") {
		t.Fatalf("status = %+v", st)
	}
	rows := a.node.Devices(false)
	if !rows[0].This || !strings.Contains(rows[0].Warning, "Can't reach the signaling servers") {
		t.Fatalf("this device's row = %+v", rows[0])
	}
	if len(rows) != 2 || rows[1].DeviceID != b.id() || rows[1].State != "offline" || !rows[1].LastSeen.IsZero() {
		t.Fatalf("the mac's row = %+v", rows)
	}
	// The log, and everything on it, still works here.
	ev := a.append(store.KindAgentSeen, map[string]any{"key": a.id() + "/s1"})
	if !a.has(ev) {
		t.Fatal("a row written with the net down is missing")
	}
}

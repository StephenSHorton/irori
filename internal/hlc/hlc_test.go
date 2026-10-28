package hlc

import (
	"testing"
	"time"
)

func TestObserveBoundedCapsTheJump(t *testing.T) {
	wall := time.UnixMilli(1_700_000_000_000)
	c := New(func() time.Time { return wall })
	bound := Timestamp(wall.Add(2*time.Minute).UnixMilli()) << 16
	c.ObserveBounded(Timestamp(wall.Add(24*time.Hour).UnixMilli())<<16, bound)
	if got := c.Now(); got <= bound || got > bound+1 {
		t.Fatalf("after a far-future observe: %d, want just past the bound %d", got, bound)
	}
	c.ObserveBounded(5, bound) // below the clock: no change
	if got := c.Now(); got != bound+2 {
		t.Fatalf("clock moved back or jumped: %d", got)
	}
}

func TestNowIncreasesWhenWallStalls(t *testing.T) {
	fixed := time.UnixMilli(1_700_000_000_000)
	c := New(func() time.Time { return fixed })
	a, b := c.Now(), c.Now()
	if b <= a {
		t.Fatalf("want b > a, got %d <= %d", b, a)
	}
	if !a.Wall().Equal(fixed) || b.Logical() != 1 {
		t.Fatalf("wall %v logical %d", a.Wall(), b.Logical())
	}
}

func TestObserveMovesPastRemote(t *testing.T) {
	c := New(func() time.Time { return time.UnixMilli(1000) })
	remote := Timestamp(5000) << 16
	c.Observe(remote)
	if got := c.Now(); got <= remote {
		t.Fatalf("want > remote, got %d", got)
	}
}

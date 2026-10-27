package hlc

import (
	"testing"
	"time"
)

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

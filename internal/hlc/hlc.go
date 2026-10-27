// Package hlc is a hybrid logical clock: wall-clock milliseconds in the high
// 48 bits and a logical counter in the low 16, so timestamps from every device
// order sensibly and never go backwards on one device.
package hlc

import (
	"sync"
	"time"
)

// Timestamp is an HLC reading. Compare with < and >; zero means unset.
type Timestamp uint64

// Clock hands out increasing timestamps. It is safe for concurrent use.
type Clock struct {
	mu   sync.Mutex
	now  func() time.Time
	last Timestamp
}

// New returns a clock that reads wall time from now (time.Now when nil).
func New(now func() time.Time) *Clock {
	if now == nil {
		now = time.Now
	}
	return &Clock{now: now}
}

// Now returns a timestamp later than every one this clock has issued or seen.
func (c *Clock) Now() Timestamp {
	c.mu.Lock()
	defer c.mu.Unlock()
	wall := Timestamp(c.now().UnixMilli()) << 16
	if wall > c.last {
		c.last = wall
	} else {
		c.last++
	}
	return c.last
}

// Observe folds in a timestamp from another device so later Now calls exceed it.
func (c *Clock) Observe(remote Timestamp) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if remote > c.last {
		c.last = remote
	}
}

// Wall is the wall-clock part of t.
func (t Timestamp) Wall() time.Time { return time.UnixMilli(int64(t >> 16)) }

// Logical is the counter part of t.
func (t Timestamp) Logical() uint16 { return uint16(t) }

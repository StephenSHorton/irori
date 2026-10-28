package coord

import (
	"context"
	"time"
)

// subBuffer is each subscriber's channel size.
const subBuffer = 64

// subscriber is one Subscribe channel's delivery state.
type subscriber struct {
	lost bool // a change was dropped; send Resync when there is room
}

// note queues c for subscribers. The caller holds c.mu; replay queues none.
func (c *coord) note(ch Change) {
	if !c.replaying {
		c.pending = append(c.pending, ch)
	}
}

// unlock hands queued changes to subscribers, re-arms the expiry timer, then
// releases c.mu. Every c.mu.Lock pairs with it.
func (c *coord) unlock() {
	defer c.mu.Unlock()
	for _, x := range c.pending {
		for ch, s := range c.subs {
			deliver(ch, s, x)
		}
	}
	clear(c.pending)
	c.pending = c.pending[:0]
	c.armExpiry()
}

// deliver sends x without blocking. A full channel marks the subscriber
// lost; its next send that fits is a Resync in place of the change. A full
// buffer still wakes the reader, and the state it then reads already has the
// dropped change.
func deliver(ch chan Change, s *subscriber, x Change) {
	if s.lost {
		x = Change{Kind: Resync}
	}
	s.lost = false
	select {
	case ch <- x:
	default:
		s.lost = true
	}
}

// armExpiry points the expiry timer at the soonest lease Until while anyone
// is subscribed, and stops it otherwise. Leases have no row when they run
// out, so this is how subscribers hear of it without a poll. The caller holds
// c.mu.
func (c *coord) armExpiry() {
	var next time.Time
	if len(c.subs) > 0 {
		for _, l := range c.held {
			if !c.localKey(l.Holder) {
				continue // another device's lease never lapses here
			}
			if next.IsZero() || l.Until.Before(next) {
				next = l.Until
			}
		}
	}
	if next.IsZero() {
		if c.expiry != nil {
			c.expiry.Stop()
		}
		c.expiryAt = time.Time{}
		return
	}
	if c.expiry != nil && next.Equal(c.expiryAt) {
		return
	}
	d := max(0, next.Sub(c.stamp()))
	if c.expiry == nil {
		c.expiry = time.AfterFunc(d, c.expire)
	} else {
		c.expiry.Stop()
		c.expiry.Reset(d)
	}
	c.expiryAt = next
}

// expire runs on the expiry timer: prune notes each lease that ran out, and
// unlock re-arms for the next one.
func (c *coord) expire() {
	c.mu.Lock()
	defer c.unlock()
	c.expiryAt = time.Time{}
	c.prune(c.stamp())
}

func (c *coord) subscribe(ctx context.Context) <-chan Change {
	ch := make(chan Change, subBuffer)
	c.mu.Lock()
	c.subs[ch] = &subscriber{}
	c.unlock()
	go func() {
		<-ctx.Done()
		c.mu.Lock()
		delete(c.subs, ch)
		c.unlock()
		close(ch)
	}()
	return ch
}

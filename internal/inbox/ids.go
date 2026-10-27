package inbox

import (
	"crypto/rand"
	"sync"
	"time"
)

// crockford is Crockford's base32 alphabet (no I, L, O, U), so ids sort as
// strings in the order they were minted.
const crockford = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"

// idGen mints ULID-style ids: 48 bits of milliseconds then 80 random bits,
// 26 base32 characters. Within one millisecond the random part counts up, so
// ids from one generator are strictly increasing.
type idGen struct {
	mu     sync.Mutex
	lastMs uint64
	last   [10]byte
}

func (g *idGen) next(now time.Time) string {
	g.mu.Lock()
	defer g.mu.Unlock()
	ms := uint64(max(now.UnixMilli(), 0))
	if ms <= g.lastMs {
		// Same (or an earlier) millisecond: keep the time part and count up.
		ms = g.lastMs
		for i := len(g.last) - 1; i >= 0; i-- {
			g.last[i]++
			if g.last[i] != 0 {
				break
			}
		}
	} else {
		g.lastMs = ms
		if _, err := rand.Read(g.last[:]); err != nil {
			panic("inbox: crypto/rand: " + err.Error())
		}
		g.last[0] &= 0x7f // leave room to count up without wrapping
	}
	var raw [16]byte
	for i := range 6 {
		raw[i] = byte(ms >> (8 * (5 - i)))
	}
	copy(raw[6:], g.last[:])
	return encode32(raw)
}

// encode32 writes 128 bits as 26 base32 characters (the first carries 3 bits).
func encode32(raw [16]byte) string {
	var out [26]byte
	// Treat raw as a 130-bit number with two leading zero bits.
	var acc uint64
	bits := 2 // the leading zero padding
	pos := 0
	for _, b := range raw {
		acc = acc<<8 | uint64(b)
		bits += 8
		for bits >= 5 {
			bits -= 5
			out[pos] = crockford[(acc>>uint(bits))&31]
			pos++
		}
	}
	return string(out[:pos])
}

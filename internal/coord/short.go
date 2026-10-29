package coord

// Short refs (docs/design/board.md "Short refs"): a card is "#12" as well as
// task:<id>, so a person can say it and type it.
//
// Every task.created row claims a number, num: one more than the highest
// number its device had folded. Two devices that create cards while they
// can't reach each other claim the same number. The fold settles it the
// same way on every device, from the rows alone: the claimants of one
// number in (hlc, device, seq) order, the first is "12", the next "12b",
// then "12c". So a card keeps its number unless a card created before it
// on another device, out of reach at the time, claimed the same one; then
// it gains a letter once (Task.ShortWas says what it was), and its number
// never moves again for that clash. Rows written before num existed claim
// their rank among such rows, in the same order.
//
// The short ref is never stored: it is derived, so it needs no owner and
// no write to converge, and a card from a device that is now offline
// settles all the same.

import (
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/store"
)

// shortClaim is one task.created row's claim on a number.
type shortClaim struct {
	id  string
	at  store.Cursor
	num int // 0: a row from before num (its claim is its rank)
}

func cursorLess(a, b store.Cursor) bool {
	if a.HLC != b.HLC {
		return a.HLC < b.HLC
	}
	if a.DeviceID != b.DeviceID {
		return a.DeviceID < b.DeviceID
	}
	return a.Seq < b.Seq
}

// shortSuffix is the letter(s) of the i-th claimant of a number (0 is
// none): b, c, ... z, then aa, ab, ...
func shortSuffix(i int) string {
	if i <= 0 {
		return ""
	}
	k := i + 1 // bijective base 26: 1 = a, 2 = b, ...
	var out []byte
	for k > 0 {
		k--
		out = append([]byte{byte('a' + k%26)}, out...)
		k /= 26
	}
	return string(out)
}

// claimShort records a task.created row's claim. A second row for the same
// task id keeps the first claim. c.mu is held.
func (c *coord) claimShort(id string, at store.Cursor, num int) {
	if !c.claimed[id] {
		c.claimed[id] = true
		c.claims = append(c.claims, shortClaim{id: id, at: at, num: max(num, 0)})
	}
	if !c.replaying { // a repeated row rebuilt the task: give it its ref back
		c.renumber()
	}
}

// renumber derives every task's short ref from the claims. c.mu is held.
// Outside replay a card whose ref changed (a clash that settled) is noted
// as changed, so readers redraw it and search re-reads it.
func (c *coord) renumber() {
	sort.Slice(c.claims, func(i, j int) bool { return cursorLess(c.claims[i].at, c.claims[j].at) })
	by := map[int][]shortClaim{}
	legacy, top := 0, 0
	for _, cl := range c.claims {
		n := cl.num
		if n == 0 {
			legacy++
			n = legacy
		}
		by[n] = append(by[n], cl) // claims are in cursor order already
		top = max(top, n)
	}
	c.shortTop = top
	c.shortIdx = make(map[string]string, len(c.claims))
	for n, cls := range by {
		base := strconv.Itoa(n)
		for i, cl := range cls {
			short := base + shortSuffix(i)
			t := c.tasks[cl.id]
			if t == nil {
				continue
			}
			c.shortIdx[short] = cl.id
			was := ""
			if i > 0 && cl.num > 0 {
				was = base
			}
			if t.Short != short && t.Short != "" && !c.replaying {
				c.shortMoved[cl.id] = c.stamp()
				c.note(Change{Kind: TaskChanged, TaskID: cl.id, Holder: t.Holder})
			}
			t.Short, t.ShortWas = short, was
		}
	}
}

// nextShort is the number a card created now claims. c.mu is held.
func (c *coord) nextShort() int { return c.shortTop + 1 }

// ParseShort reads a short ref: "#12", "12", "#12b" (any case, spaces
// trimmed). ok is false for anything else, a full task id included.
func ParseShort(ref string) (short string, ok bool) {
	s := strings.ToLower(strings.TrimSpace(ref))
	s = strings.TrimPrefix(s, "#")
	if s == "" || len(s) > 12 {
		return "", false
	}
	i := 0
	for i < len(s) && s[i] >= '0' && s[i] <= '9' {
		i++
	}
	if i == 0 || s[0] == '0' {
		return "", false
	}
	for _, r := range s[i:] {
		if r < 'a' || r > 'z' {
			return "", false
		}
	}
	return s, true
}

// TaskRef resolves what someone typed for a card to its task id: a short
// ref ("#12", "12b", "task:#12") or a full id, with or without "task:".
// A short ref no card has is ErrNotFound; a full id is returned as given
// (callers that act on it report a missing one).
func (c *Coord) TaskRef(ref string) (string, error) { return c.impl.taskRef(ref) }

func (c *coord) taskRef(ref string) (string, error) {
	v := strings.TrimSpace(ref)
	if k, id, ok := strings.Cut(v, ":"); ok && strings.EqualFold(strings.TrimSpace(k), "task") {
		v = strings.TrimSpace(id)
	}
	short, ok := ParseShort(v)
	if !ok {
		return v, nil
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if id := c.shortIdx[short]; id != "" {
		return id, nil
	}
	if c.tasks[v] != nil {
		return v, nil // an id that happens to read like a number
	}
	return "", ErrNotFound
}

// ShortRef is how a card is named to people and agents: "#12", or
// task:<id> for a task the fold hasn't numbered.
func (t Task) ShortRef() string {
	if t.Short == "" {
		return "task:" + t.ID
	}
	return "#" + t.Short
}

// Refs is the short ref then the full one, "#12, task:<id>", for text
// that names a card once and should still find it after a renumber.
func (t Task) Refs() string {
	if t.Short == "" {
		return "task:" + t.ID
	}
	return "#" + t.Short + ", task:" + t.ID
}

// movedSince reports whether id's short ref changed after since (search
// re-reads those docs). c.mu is held.
func (c *coord) movedSince(id string, since time.Time) bool {
	at, ok := c.shortMoved[id]
	return ok && !at.Before(since)
}

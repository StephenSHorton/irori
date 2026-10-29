package coord

// Short refs (docs/design/board.md "Short refs"): a card is "#12" as well as
// task:<id>, so a person can say it and type it.
//
// Every task.created row claims a number, num: one more than the highest
// number its device had folded. Two devices that create cards while they
// can't reach each other claim the same number. The fold settles it the
// same way on every device, from the rows alone: the claimants of one
// number in (hlc, device, seq) order become "12a", "12b", "12c", and each
// says it was 12 (Task.ShortWas). "12" alone then names none of them: a
// person or agent who saw "#12" on one device may mean either, so taskRef
// refuses it and names both, rather than acting on the card they didn't
// mean. So a card keeps its number unless a card made on another device,
// out of reach at the time, claimed the same one; then it gains a letter,
// and further claims on 12 only add letters after it. Rows written before
// num existed claim their rank among such rows, in the same order.
//
// The short ref is never stored: it is derived, so it needs no owner and
// no write to converge, and a card from a device that is now offline
// settles all the same.

import (
	"fmt"
	"slices"
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

// shortSuffix is the letter(s) of the i-th claimant of a number that has
// more than one: a, b, ... z, then aa, ab, ...
func shortSuffix(i int) string {
	k := max(i, 0) + 1 // bijective base 26: 1 = a, 2 = b, ...
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
//
// Outside replay only the claimed number's group is settled again (a bulk
// sync folds one row at a time under c.mu, so a full renumber per row would
// be quadratic); a row from before num, whose number is its rank, and a
// repeated row renumber everything.
func (c *coord) claimShort(id string, at store.Cursor, num int) {
	if c.claimed[id] {
		if !c.replaying { // a repeated row rebuilt the task: give it its ref back
			c.renumber()
		}
		return
	}
	cl := shortClaim{id: id, at: at, num: max(num, 0)}
	c.claimed[id] = true
	c.claims = append(c.claims, cl)
	if c.replaying {
		return
	}
	if cl.num == 0 {
		c.renumber()
		return
	}
	g := c.shortGroups[cl.num]
	i := sort.Search(len(g), func(i int) bool { return cursorLess(cl.at, g[i].at) })
	c.shortGroups[cl.num] = slices.Insert(g, i, cl)
	c.shortTop = max(c.shortTop, cl.num)
	c.settle(cl.num)
}

// renumber derives every task's short ref from the claims. c.mu is held.
// shortTop only rises: a re-fold after a revoke's cut drops claims, and the
// numbers they had aren't handed out again while this device runs.
func (c *coord) renumber() {
	sort.Slice(c.claims, func(i, j int) bool { return cursorLess(c.claims[i].at, c.claims[j].at) })
	c.shortGroups = map[int][]shortClaim{}
	legacy := 0
	for _, cl := range c.claims {
		n := cl.num
		if n == 0 {
			legacy++
			n = legacy
		}
		c.shortGroups[n] = append(c.shortGroups[n], cl) // claims are in cursor order already
		c.shortTop = max(c.shortTop, n)
	}
	c.shortIdx = make(map[string]string, len(c.claims))
	for n := range c.shortGroups {
		c.settle(n)
	}
}

// settle gives number n's claimants their refs: a lone claimant is "12";
// once two or more claimed 12, each gets a letter in cursor order (12a,
// 12b, ...), says it was 12, and "12" alone names none of them (taskRef
// refuses it with both named). Outside replay a card whose ref changed
// is noted as moved. c.mu is held.
func (c *coord) settle(n int) {
	g := c.shortGroups[n]
	base := strconv.Itoa(n)
	delete(c.shortIdx, base)
	for i, cl := range g {
		short, was := base, ""
		if len(g) > 1 {
			short, was = base+shortSuffix(i), base
		}
		t := c.tasks[cl.id]
		if t == nil {
			continue
		}
		c.shortIdx[short] = cl.id
		if t.Short != short && t.Short != "" && !c.replaying {
			c.shortMovedTo(t, t.Short, short)
		}
		t.Short, t.ShortWas = short, was
	}
}

// shortMovedTo notes that t's short ref moved from one to another, outside
// replay: readers redraw it, search re-reads it, and its watchers, holders
// and creator hear about it in their watch digest (no activity line: a
// replay wouldn't write one). c.mu is held.
func (c *coord) shortMovedTo(t *Task, from, to string) {
	c.shortMoved[t.ID] = c.stamp()
	c.note(Change{Kind: TaskChanged, TaskID: t.ID, Holder: t.Holder})
	what := "renumbered it #" + to + " (it was #" + from + ")"
	if num := shortNum(to); num != to {
		what += ": more than one card made on devices that were apart claimed #" + num + ", so #" + num + " alone names none of them now"
	}
	c.queueWatch(t, sweeper, what, append(slices.Clone(t.Holders), t.CreatedBy)...)
}

// shortNum is a short ref's number without its letters ("12b" is "12").
func shortNum(short string) string { return strings.TrimRight(short, "abcdefghijklmnopqrstuvwxyz") }

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
// A short ref no card has is ErrNotFound; a number two or more cards
// claimed apart ("12" once there are 12a and 12b) is ErrAmbiguous, and its
// text names each of them; a full id is returned as given (callers that
// act on it report a missing one).
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
	if err := c.ambiguous(short); err != nil {
		return "", err
	}
	return "", ErrNotFound
}

// ambiguous is ErrAmbiguous, naming each card, when short is a bare number
// more than one card claimed; nil otherwise. c.mu is held.
func (c *coord) ambiguous(short string) error {
	n, err := strconv.Atoi(short)
	if err != nil || len(c.shortGroups[n]) < 2 {
		return nil
	}
	var names []string
	for _, cl := range c.shortGroups[n] {
		if t := c.tasks[cl.id]; t != nil {
			names = append(names, fmt.Sprintf("#%s %q (task:%s)", t.Short, oneLineTitle(t.Title), t.ID))
		}
	}
	return fmt.Errorf("%w: cards made on devices that were apart each claimed #%s, so it is now %s", ErrAmbiguous, short, strings.Join(names, ", "))
}

// oneLineTitle is a title cut to one short line for an error's text.
func oneLineTitle(s string) string {
	s = strings.Join(strings.Fields(s), " ")
	if r := []rune(s); len(r) > 60 {
		s = string(r[:59]) + "…"
	}
	return s
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

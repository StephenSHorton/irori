package coord

import (
	"strings"

	"github.com/StephenSHorton/irori/internal/store"
)

// A coord over a shared board's log (design/v0.5-shared-boards.md §2.1,
// §2.4): Options.Board. The fold is the private one, with three changes:
//
//   - callers are board keys, "person:<personId>" and "agent:<cardId>", and
//     each is its row's actor as given (actorFor), never "user";
//   - a person's row is one whose actor is that person:<id> key and which
//     Options.IsOwner accepts (personRow), where the private log asks for
//     actor "user" and a "user:<device>" key;
//   - a row whose references don't resolve in the log yet (a forwarded row
//     that arrived before what it names, §2.3) is set aside, re-checked
//     whenever another row folds, and never applied until it resolves.
//
// The Core translates its callers' private keys into board keys before it
// calls a board coord (core/boards.go).

// personRow reports a row the person wrote as themselves: actor "user" and
// an owner's "user:<device>" by in the private log; actor and by the same
// owning person:<id> key on a board.
func (c *coord) personRow(ev store.Event, by string) bool {
	if c.onBoard {
		return ev.Actor == by && strings.HasPrefix(by, store.PersonActorPrefix) && c.isOwner(by)
	}
	return ev.Actor == "user" && c.isOwner(by)
}

// personActor reports a row whose actor is a person (whoever its by says).
func (c *coord) personActor(ev store.Event) bool {
	if c.onBoard {
		return strings.HasPrefix(ev.Actor, store.PersonActorPrefix)
	}
	return ev.Actor == "user"
}

// selfKey is the person at this device as a caller: "user:<device>", or
// on a board Options.Person.
func (c *coord) selfKey() string {
	if c.onBoard {
		return c.person
	}
	return "user:" + c.st.DeviceID()
}

// asideGate sets ev aside when it names something the board log doesn't
// hold yet; true means fold it now. c.mu is held.
func (c *coord) asideGate(ev store.Event) bool {
	if !c.onBoard || c.st.Unresolved(ev) == nil {
		return true
	}
	c.aside = append(c.aside, ev)
	return false
}

// retryAside folds every set-aside row that resolves now, until a pass
// folds none (one may resolve another). c.mu is held.
func (c *coord) retryAside() {
	for len(c.aside) > 0 {
		var keep []store.Event
		folded := false
		for _, ev := range c.aside {
			if c.st.Unresolved(ev) != nil {
				keep = append(keep, ev)
				continue
			}
			folded = true
			c.fold(ev)
		}
		c.aside = keep
		if !folded {
			return
		}
	}
}

// Unresolved counts the board rows set aside because what they name isn't
// in the log yet (§2.3); always 0 for the private log.
func (c *Coord) Unresolved() int {
	c.impl.mu.Lock()
	defer c.impl.mu.Unlock()
	return len(c.impl.aside)
}

// IsBoard reports a coord over a shared board's log.
func (c *Coord) IsBoard() bool { return c.impl.onBoard }

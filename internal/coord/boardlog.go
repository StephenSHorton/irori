package coord

import (
	"context"
	"fmt"
	"strings"
	"unicode/utf8"

	"github.com/StephenSHorton/irori/internal/owner"
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

// mayWrite is the owner rule for an owner-only row (v0.4 §6.0): in the
// private log the owner key's device wrote it; on a board, a device of the
// owner key's person did (§2.4), since a card hides which device its agent
// runs on. A key whose person isn't known, or a writer certified to no
// member, owns nothing.
func (c *coord) mayWrite(ownerKey, writer string) bool {
	if !c.onBoard || c.keyPerson == nil || c.devPerson == nil {
		return owner.MayWrite(ownerKey, writer)
	}
	p := c.keyPerson(ownerKey)
	return p != "" && p == c.devPerson(writer)
}

// tag is the board's short handle (Options.Tag), "" on the private board.
func (c *coord) tag() string {
	if !c.onBoard || c.tagFn == nil {
		return ""
	}
	return c.tagFn()
}

// mayCancel is §5.3's rule for a cancel on a shared card, withdrawn or
// not: it counts only from a device of the card creator's person or of an
// owner, so another member can't cancel your card. The private log has no
// such rule; nor does a board coord without the owner rule's functions.
func (c *coord) mayCancel(t *Task, writer string) bool {
	if !c.onBoard || c.keyPerson == nil || c.devPerson == nil {
		return true
	}
	p := c.devPerson(writer)
	if p == "" {
		return false
	}
	return p == c.keyPerson(t.CreatedBy) || (c.boardOwner != nil && c.boardOwner(p))
}

// Withdraw takes a card back off a shared board (v0.5 §5.3): canceled with
// withdrawn set, by its creator's person or an owner. Nothing is recalled:
// other members' devices keep what was shared, and hide the card.
func (c *Coord) Withdraw(ctx context.Context, by, id, note string) (Task, error) {
	return c.impl.withdraw(ctx, by, id, note)
}

func (c *coord) withdraw(ctx context.Context, by, id, note string) (Task, error) {
	by, note = strings.TrimSpace(by), strings.TrimSpace(note)
	if !c.onBoard {
		return Task{}, fmt.Errorf("%w: only a card on a shared board is withdrawn; cancel a private card instead", ErrInvalid)
	}
	if utf8.RuneCountInString(note) > maxObjective {
		return Task{}, fmt.Errorf("%w: note longer than %d characters", ErrInvalid, maxObjective)
	}
	c.mu.Lock()
	defer c.unlock()
	t := c.tasks[id]
	if t == nil {
		return Task{}, fmt.Errorf("%w: task %q", ErrNotFound, id)
	}
	if t.Withdrawn {
		return copyTask(t), nil
	}
	if !c.mayCancel(t, c.st.DeviceID()) {
		return Task{}, fmt.Errorf("%w: only its creator or a board owner withdraws %s", ErrNotAllowed, t.ShortRef())
	}
	note = cmpStr(note, "withdrawn from the board")
	if err := c.write(ctx, by, KindTaskState, taskStateRow{TaskID: id, State: Canceled, Note: note, Withdrawn: true, By: by, At: c.stamp()}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
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

package coord

import (
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/store"
)

// Pins (docs/design/board.md "Pinned cards"): the person pins the cards
// that matter most, and the window shows them in their own group, in their
// own order. A pin is the person's: only a task.pinned row with actor
// "user" by an owner key counts, and it replicates to the person's devices.
// For one card the row latest in (hlc, device, seq) order stands.

// KindTaskPinned pins or unpins a card: {taskId, pinned, rank, by}. rank
// orders the pinned cards, lowest first.
const KindTaskPinned = "task.pinned"

type taskPinnedRow struct {
	TaskID string    `json:"taskId"`
	Pinned bool      `json:"pinned"`
	Rank   float64   `json:"rank,omitempty"`
	By     string    `json:"by"`
	At     time.Time `json:"at"`
}

type pinRec struct {
	pinned bool
	rank   float64
	at     store.Cursor
}

// foldPin applies a task.pinned row: only the person's, and only when it
// comes after the card's current pin row. c.mu is held.
func (c *coord) foldPin(ev store.Event) {
	var p taskPinnedRow
	if json.Unmarshal(ev.Payload, &p) != nil || p.TaskID == "" {
		return
	}
	if ev.Actor != "user" || !c.isOwner(p.By) {
		c.ignore(ev.DeviceID)
		return
	}
	at := ev.Cursor()
	if old, ok := c.pins[p.TaskID]; ok && !cursorLess(old.at, at) {
		return
	}
	c.pins[p.TaskID] = pinRec{pinned: p.Pinned, rank: p.Rank, at: at}
	if t := c.tasks[p.TaskID]; t != nil {
		c.syncPin(t)
		c.note(Change{Kind: TaskChanged, TaskID: t.ID})
	}
}

// syncPin copies the card's pin onto it. c.mu is held.
func (c *coord) syncPin(t *Task) {
	r := c.pins[t.ID]
	t.Pinned, t.PinRank = r.pinned, 0
	if r.pinned {
		t.PinRank = r.rank
	}
}

// Pin pins (or with pinned false unpins) card id for the person by. A new
// pin goes after every pinned card.
func (c *Coord) Pin(ctx context.Context, by, id string, pinned bool) (Task, error) {
	return c.impl.pin(ctx, by, id, pinned, "", false)
}

// MovePin moves pinned card id to just before pinned card before ("" puts
// it last).
func (c *Coord) MovePin(ctx context.Context, by, id, before string) (Task, error) {
	return c.impl.pin(ctx, by, id, true, before, true)
}

func (c *coord) pin(ctx context.Context, by, id string, pinned bool, before string, move bool) (Task, error) {
	by = strings.TrimSpace(by)
	if !c.isOwner(by) {
		return Task{}, fmt.Errorf("%w: only the person pins cards", ErrNotAllowed)
	}
	c.mu.Lock()
	defer c.unlock()
	t := c.tasks[id]
	if t == nil {
		return Task{}, fmt.Errorf("%w: task %q", ErrNotFound, id)
	}
	if !move && t.Pinned == pinned {
		return copyTask(t), nil
	}
	rank := 0.0
	if pinned {
		var err error
		if rank, err = c.pinRank(id, before, move); err != nil {
			return Task{}, err
		}
	}
	if err := c.write(ctx, by, KindTaskPinned, taskPinnedRow{TaskID: id, Pinned: pinned, Rank: rank, By: by, At: c.stamp()}); err != nil {
		return Task{}, err
	}
	return copyTask(t), nil
}

// pinRank is where id goes among the pinned cards: last, or just before
// before. c.mu is held.
func (c *coord) pinRank(id, before string, move bool) (float64, error) {
	type pr struct {
		id   string
		rank float64
	}
	var list []pr
	for tid, r := range c.pins {
		if r.pinned && tid != id {
			list = append(list, pr{tid, r.rank})
		}
	}
	sort.Slice(list, func(i, j int) bool {
		if list[i].rank != list[j].rank {
			return list[i].rank < list[j].rank
		}
		return list[i].id < list[j].id
	})
	if !move || before == "" {
		if len(list) == 0 {
			return 1, nil
		}
		return list[len(list)-1].rank + 1, nil
	}
	i := slices.IndexFunc(list, func(p pr) bool { return p.id == before })
	if i < 0 {
		return 0, fmt.Errorf("%w: %s isn't pinned", ErrInvalid, before)
	}
	if i == 0 {
		return list[0].rank - 1, nil
	}
	return (list[i-1].rank + list[i].rank) / 2, nil
}

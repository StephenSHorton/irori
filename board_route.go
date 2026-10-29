package main

import (
	"context"
	"fmt"

	"github.com/StephenSHorton/irori/internal/coord"
)

// taskRouter is core.Core.TaskCoord: the coord of the log a card lives in
// (the private board or a shared one, v0.5 §2.1) and each key as that log
// knows it.
type taskRouter func(ctx context.Context, taskID string, keys ...string) (*coord.Coord, []string, error)

// on is the coord the person's action on card id goes to, with the
// person's key first in the keys returned and then each of more, as that
// log knows them. With no router there is the private board alone.
func (b *BoardService) on(ctx context.Context, id string, more ...string) (*coord.Coord, []string, error) {
	keys := append([]string{b.user.Key}, more...)
	if b.router == nil {
		return b.coord, keys, nil
	}
	return b.router(ctx, id, keys...)
}

// read is the coord holding card id, for reads: no key is translated, so
// nothing is written.
func (b *BoardService) read(id string) *coord.Coord {
	if b.router == nil {
		return b.coord
	}
	co, _, err := b.router(context.Background(), id)
	if err != nil || co == nil {
		return b.coord
	}
	return co
}

// privateOnly is on for an action kept on the private board alone (pins):
// a shared card is refused rather than written to its board.
func (b *BoardService) privateOnly(ctx context.Context, id string) (*coord.Coord, []string, error) {
	co, as, err := b.on(ctx, id)
	if err != nil {
		return nil, nil, err
	}
	if co != b.coord {
		return nil, nil, fmt.Errorf("%w: a card on a shared board isn't pinned", coord.ErrInvalid)
	}
	return co, as, nil
}

package main

import (
	"context"

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

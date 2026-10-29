package tools

import (
	"context"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
)

// TaskRouter is a Backend that holds more than one log (v0.5 §2.1): the
// private board and each shared board, with a coord each. core.Core
// implements it; a Backend without it has the private log only.
type TaskRouter interface {
	// TaskCoord is the coord of the log taskID lives in, and each of keys
	// as the caller is known there (a board key on a shared board). An id
	// in no log gets the private coord, which reports it not found.
	TaskCoord(ctx context.Context, taskID string, keys ...string) (*coord.Coord, []string, error)
	// PrivateNotice gives a board coord's notice the keys this device's
	// inbox knows.
	PrivateNotice(n coord.Notice) coord.Notice
}

// on is the coord a call on card id acts on, with keys as it knows them.
func (s *set) on(ctx context.Context, id string, keys ...string) (*coord.Coord, []string, error) {
	if r, ok := s.b.(TaskRouter); ok {
		return r.TaskCoord(ctx, id, keys...)
	}
	return s.b.Coord(), keys, nil
}

// tell routes a coord notice from the caller (RouteNotice), its keys made
// private first when it came from a board's coord.
func (s *set) tell(ctx context.Context, from inbox.Sender, n coord.Notice) []NoticeResult {
	if r, ok := s.b.(TaskRouter); ok {
		n = r.PrivateNotice(n)
	}
	return RouteNotice(ctx, s.b.Inbox(), from, n)
}

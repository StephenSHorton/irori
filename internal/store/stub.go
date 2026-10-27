package store

import (
	"context"

	"github.com/StephenSHorton/irori/internal/hlc"
)

// Skeleton stubs. The store work package replaces this file.

type db struct{}

func open(context.Context, string, string, *hlc.Clock) (*Store, error) { return nil, ErrUnimplemented }
func (s *Store) close() error                                          { return nil }
func (s *Store) append(context.Context, string, string, any) (Event, error) {
	return Event{}, ErrUnimplemented
}
func (s *Store) insert(context.Context, Event) error    { return ErrUnimplemented }
func (s *Store) vector(context.Context) (Vector, error) { return nil, ErrUnimplemented }
func (s *Store) missing(context.Context, Vector, int) ([]Event, error) {
	return nil, ErrUnimplemented
}
func (s *Store) list(context.Context, Filter) ([]Event, error) { return nil, ErrUnimplemented }
func (s *Store) subscribe(ctx context.Context) <-chan Event {
	ch := make(chan Event)
	go func() { <-ctx.Done(); close(ch) }()
	return ch
}

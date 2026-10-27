package inbox

import (
	"context"

	"github.com/StephenSHorton/irori/internal/store"
)

// Skeleton stubs. The inbox work package replaces this file.

type inbox struct{}

func newInbox(context.Context, *store.Store, Roster, Options) (*Inbox, error) {
	return &Inbox{impl: &inbox{}}, nil
}
func (b *inbox) send(context.Context, Sender, Address, string, string) (Message, Delivery, error) {
	return Message{}, Delivery{}, ErrUnimplemented
}
func (b *inbox) reply(context.Context, Sender, string, string) (Message, Delivery, error) {
	return Message{}, Delivery{}, ErrUnimplemented
}
func (b *inbox) setReachable(string, Path, bool)                           {}
func (b *inbox) nextWake(context.Context, string, Path) (*Wake, error)     { return nil, nil }
func (b *inbox) markDelivered(context.Context, string, string) error       { return ErrUnimplemented }
func (b *inbox) markEngaged(context.Context, string, string, string) error { return ErrUnimplemented }
func (b *inbox) turnEnded(context.Context, string, string, string) error   { return ErrUnimplemented }
func (b *inbox) ack(context.Context, string, string) error                 { return ErrUnimplemented }
func (b *inbox) deferDelivery(context.Context, string, string, int, string) error {
	return ErrUnimplemented
}
func (b *inbox) skip(context.Context, string, string, string) error { return ErrUnimplemented }
func (b *inbox) release(context.Context, string, string) error      { return ErrUnimplemented }
func (b *inbox) dismiss(context.Context, string, string) error      { return ErrUnimplemented }
func (b *inbox) forAgent(string, bool, int) []Delivery              { return nil }
func (b *inbox) list(int) []Delivery                                { return nil }
func (b *inbox) get(string) (Delivery, bool)                        { return Delivery{}, false }
func (b *inbox) policy() Policy                                     { return Policy{Mode: Everyone} }
func (b *inbox) setPolicy(context.Context, Policy, string) error    { return ErrUnimplemented }
func (b *inbox) sweep(context.Context) error                        { return nil }
func (b *inbox) subscribe(ctx context.Context) <-chan Change {
	ch := make(chan Change)
	go func() { <-ctx.Done(); close(ch) }()
	return ch
}
func fencePrompt(string, bool, []Delivery) string { return "" }

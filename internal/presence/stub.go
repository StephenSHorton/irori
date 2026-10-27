package presence

import "context"

// Skeleton stubs. The presence work package replaces this file.

type watcher struct{}

func newWatcher(Options) *Watcher                { return &Watcher{impl: &watcher{}} }
func (w *Watcher) run(ctx context.Context) error { <-ctx.Done(); return nil }
func (w *Watcher) snapshot() []AgentCard         { return nil }
func (w *Watcher) subscribe(ctx context.Context) <-chan Change {
	ch := make(chan Change)
	go func() { <-ctx.Done(); close(ch) }()
	return ch
}

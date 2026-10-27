package capture

import (
	"context"

	"github.com/StephenSHorton/irori/internal/decision"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/store"
)

// stub: replaced by the capture work package.
type capture struct{}

func newCapture(ctx context.Context, st *store.Store, roster Roster, opts Options) (*Capture, error) {
	return &Capture{impl: &capture{}}, nil
}
func (c *capture) run(ctx context.Context) error                  { <-ctx.Done(); return nil }
func (c *capture) timeline(q TimelineQuery) Timeline              { return Timeline{Key: q.Key} }
func (c *capture) edits(q EditQuery) []EditRow                    { return nil }
func (c *capture) commits(repoKey string, limit int) []CommitView { return nil }
func (c *capture) commit(sha string) (CommitView, bool)           { return CommitView{}, false }
func (c *capture) repos() map[string][]string                     { return nil }
func (c *capture) blob(id string) ([]byte, error)                 { return nil, ErrNotFound }
func (c *capture) prOwners(prKey string) []string                 { return nil }
func (c *capture) prs(repo string) []ledger.Entry                 { return nil }
func (c *capture) why(ctx context.Context, q WhyQuery) (WhyResult, error) {
	return WhyResult{}, ErrUnimplemented
}
func (c *capture) recordDecision(ctx context.Context, in DecisionInput) (decision.Decision, error) {
	return decision.Decision{}, ErrUnimplemented
}
func (c *capture) decisions(f decision.Filter) []DecisionView  { return nil }
func (c *capture) subscribe(ctx context.Context) <-chan Change { return make(chan Change) }
func (c *capture) status() Status                              { return Status{} }

package inbox

// Search (search.Source; CONTRACT, implemented by the inbox work package,
// A2). The inbox owns "msg:<messageId>" refs. A message's Doc: Title "<from
// name> -> <recipient name or repo>" plus its Reason when set, Body = its
// text, At = SentAt, Agent = the sender's key when an agent (else the first
// recipient's), RepoKey = the repo it was addressed to, if any. Messages
// never change, so Updated = SentAt, except that a message becomes
// searchable only once some delivery of it has left Held: a message that is
// still held, or was dismissed while held, is never in SearchDocs and
// SearchShow answers search.ErrNotFound for it (search must not get past the
// policy). A released message comes back with Updated = its release time.
// Show renders the fenced-style header (from, to, trust, ref, state of each
// delivery) and the text; Related: the message it replies to, its replies,
// and Ref when set.

import (
	"context"
	"time"

	"github.com/StephenSHorton/irori/internal/search"
)

// SearchKinds implements search.Source.
func (b *Inbox) SearchKinds() []search.Kind { return []search.Kind{search.KindMsg} }

// SearchDocs implements search.Source.
func (b *Inbox) SearchDocs(ctx context.Context, since time.Time, limit int) ([]search.Doc, error) {
	return nil, nil
}

// SearchShow implements search.Source.
func (b *Inbox) SearchShow(ctx context.Context, ref string, o search.ShowOptions) (search.Shown, error) {
	return search.Shown{}, ErrUnimplemented
}

var _ search.Source = (*Inbox)(nil)

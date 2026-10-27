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
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/search"
)

// maxSearchDocs bounds one SearchDocs page when the caller gives no limit.
const maxSearchDocs = 500

// SearchKinds implements search.Source.
func (b *Inbox) SearchKinds() []search.Kind { return []search.Kind{search.KindMsg} }

// SearchDocs implements search.Source.
func (b *Inbox) SearchDocs(ctx context.Context, since time.Time, limit int) ([]search.Doc, error) {
	if limit <= 0 {
		limit = maxSearchDocs
	}
	x := b.impl
	x.mu.Lock()
	defer x.mu.Unlock()
	byMsg := map[string][]*drec{}
	for _, did := range x.order {
		if d := x.dels[did]; d != nil {
			byMsg[d.MessageID] = append(byMsg[d.MessageID], d)
		}
	}
	var out []search.Doc
	for id, m := range x.msgs {
		up, ok := x.searchable(id)
		if !ok || up.Before(since) {
			continue
		}
		out = append(out, x.doc(m, up, byMsg[id]))
	}
	slices.SortFunc(out, func(p, q search.Doc) int {
		if c := p.Updated.Compare(q.Updated); c != 0 {
			return c
		}
		return strings.Compare(p.Ref, q.Ref)
	})
	if len(out) > limit {
		out = out[:limit]
	}
	return out, ctx.Err()
}

// SearchShow implements search.Source.
func (b *Inbox) SearchShow(ctx context.Context, ref string, o search.ShowOptions) (search.Shown, error) {
	kind, id, err := search.ParseRef(ref)
	if err != nil {
		return search.Shown{}, err
	}
	if kind != search.KindMsg {
		return search.Shown{}, fmt.Errorf("%w: %s", search.ErrNotFound, ref)
	}
	x := b.impl
	x.mu.Lock()
	defer x.mu.Unlock()
	m := x.msgs[id]
	up, ok := x.searchable(id)
	if m == nil || !ok {
		return search.Shown{}, fmt.Errorf("%w: %s", search.ErrNotFound, ref)
	}
	dels := x.deliveriesOf(id)
	doc := x.doc(m, up, dels)
	full := x.render(m, doc.Title, dels)
	text, more, next := search.Page(full, o)
	return search.Shown{
		Ref: doc.Ref, Kind: search.KindMsg, Title: doc.Title,
		Text: text, More: more, Next: next, Related: x.related(m),
	}, ctx.Err()
}

// searchable reports whether message id may be found, and since when (its
// Updated): SentAt, or the time it first left Held when it was held first.
func (b *inbox) searchable(id string) (time.Time, bool) {
	m := b.msgs[id]
	at, ok := b.visibleAt[id]
	if m == nil || !ok {
		return time.Time{}, false
	}
	if !b.heldFirst[id] || at.Before(m.SentAt) {
		return m.SentAt, true
	}
	return at, true
}

// deliveriesOf lists message id's deliveries, oldest first.
func (b *inbox) deliveriesOf(id string) []*drec {
	var out []*drec
	for _, did := range b.order {
		if d := b.dels[did]; d != nil && d.MessageID == id {
			out = append(out, d)
		}
	}
	return out
}

// nameOf is key's display name without the key ("" when unknown).
func (b *inbox) nameOf(key string) string {
	switch {
	case key == SystemKey:
		return "Irori"
	case key == "":
		return ""
	}
	if n := b.known[key]; n != "" {
		return n
	}
	if a := b.aliases[key]; len(a) > 0 {
		return a[len(a)-1]
	}
	return key
}

// recipient names where m went, for a title.
func (b *inbox) recipient(m *Message, dels []*drec) string {
	switch m.To.Kind {
	case ToRepo:
		return "repo " + m.To.Value
	case ToPR:
		return m.To.Value
	}
	if len(dels) > 0 && dels[0].To != "" {
		return b.nameOf(dels[0].To)
	}
	return m.To.Value
}

// doc is m as the index sees it; dels are its deliveries, oldest first.
func (b *inbox) doc(m *Message, updated time.Time, dels []*drec) search.Doc {
	title := fmt.Sprintf("%s -> %s", m.From.Name, b.recipient(m, dels))
	if m.Reason != "" {
		title += ": " + m.Reason
	}
	d := search.Doc{
		Ref: search.Ref(search.KindMsg, m.ID), Kind: search.KindMsg,
		At: m.SentAt, Updated: updated, Title: oneLine(title), Body: m.Text,
	}
	switch {
	case m.From.Key != "" && !m.From.IsUser() && !m.From.isSystem():
		d.Agent = m.From.Key
	case len(dels) > 0 && !isUserKey(dels[0].To):
		d.Agent = dels[0].To
	}
	switch m.To.Kind {
	case ToRepo:
		d.RepoKey = m.To.Value
	case ToPR:
		if ref, ok := ledger.ParseTarget(m.To.Value); ok {
			d.RepoKey = ledger.RepoKey(ref.Repo)
		}
	}
	return d
}

// render is a message's detail: the fenced-style header, each delivery's
// state, then the text.
func (b *inbox) render(m *Message, title string, dels []*drec) string {
	var sb strings.Builder
	fmt.Fprintf(&sb, "%s\n", title)
	fmt.Fprintf(&sb, "from: %s (%s) trust=%s\n", m.From.Name, m.From.Key, m.Trust)
	fmt.Fprintf(&sb, "to: %s %s\n", m.To.Kind, m.To.Value)
	fmt.Fprintf(&sb, "sent: %s\n", m.SentAt.UTC().Format(time.RFC3339))
	if m.Reason != "" {
		fmt.Fprintf(&sb, "reason: %s\n", oneLine(m.Reason))
	}
	if m.ReplyTo != "" {
		fmt.Fprintf(&sb, "reply to: %s\n", search.Ref(search.KindMsg, m.ReplyTo))
	}
	if m.Ref != "" {
		fmt.Fprintf(&sb, "about: %s\n", m.Ref)
	}
	if m.Notice {
		sb.WriteString("notice: from the Irori Board, needs no reply\n")
	}
	sb.WriteString("deliveries:\n")
	for _, d := range dels {
		who := "nobody yet"
		if d.To != "" {
			who = fmt.Sprintf("%s (%s)", b.nameOf(d.To), d.To)
		}
		fmt.Fprintf(&sb, "  %s to %s: %s", d.ID, who, d.State)
		if d.Hold != "" {
			fmt.Fprintf(&sb, " (%s)", d.Hold)
		}
		if d.Mode == ModeNextTurn {
			sb.WriteString(", next turn")
		}
		if d.Note != "" {
			fmt.Fprintf(&sb, ", note: %s", oneLine(d.Note))
		}
		sb.WriteString("\n")
	}
	sb.WriteString("\n")
	sb.WriteString(m.Text)
	sb.WriteString("\n")
	return sb.String()
}

// related: the message m answers, its searchable replies, and its Ref.
func (b *inbox) related(m *Message) []string {
	var out []string
	if m.ReplyTo != "" {
		if _, ok := b.searchable(m.ReplyTo); ok {
			out = append(out, search.Ref(search.KindMsg, m.ReplyTo))
		}
	}
	if m.Ref != "" {
		out = append(out, m.Ref)
	}
	var replies []string
	for id, r := range b.msgs {
		if r.ReplyTo == m.ID {
			if _, ok := b.searchable(id); ok {
				replies = append(replies, id)
			}
		}
	}
	slices.Sort(replies)
	for _, id := range replies {
		out = append(out, search.Ref(search.KindMsg, id))
	}
	if len(out) > search.MaxRelated {
		out = out[:search.MaxRelated]
	}
	return out
}

// oneLine folds s onto one line.
func oneLine(s string) string { return strings.Join(strings.Fields(s), " ") }

var _ search.Source = (*Inbox)(nil)

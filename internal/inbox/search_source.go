package inbox

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/search"
)

// The search.Source behind Inbox.SearchDocs and SearchShow (search.go).

// visible reports whether a delivery lets its message be found: it has left
// Held and wasn't dismissed while held (Dismiss only applies to held ones).
func visible(d *drec) bool { return d.State != Held && d.State != Dismissed }

// msgDels lists each message's deliveries, in delivery order; x.mu is held.
func (x *inbox) msgDels() map[string][]*drec {
	out := map[string][]*drec{}
	for _, id := range x.order {
		if d := x.dels[id]; d != nil {
			out[d.MessageID] = append(out[d.MessageID], d)
		}
	}
	return out
}

// msgDoc builds a message's doc, ok false while no delivery is visible.
// Updated is the newest change of a visible delivery (at least SentAt), so
// a message released from a hold comes back at its release; x.mu is held.
func (x *inbox) msgDoc(m *Message, dels []*drec) (search.Doc, bool) {
	upd := m.SentAt
	seen := false
	for _, d := range dels {
		if !visible(d) {
			continue
		}
		seen = true
		if d.UpdatedAt.After(upd) {
			upd = d.UpdatedAt
		}
	}
	if !seen {
		return search.Doc{}, false
	}
	title := x.senderName(m) + " -> " + x.recipientName(m, dels)
	if m.Reason != "" {
		title += " · " + m.Reason
	}
	agent := ""
	if fromAgent(m) {
		agent = m.From.Key
	} else {
		for _, d := range dels {
			if visible(d) && d.To != "" {
				agent = d.To
				break
			}
		}
	}
	repo := ""
	if m.To.Kind == ToRepo {
		repo = m.To.Value
	} else {
		for _, d := range dels {
			if d.RepoKey != "" {
				repo = d.RepoKey
				break
			}
		}
	}
	return search.Doc{
		Ref: search.Ref(search.KindMsg, m.ID), Kind: search.KindMsg, At: m.SentAt, Updated: upd,
		Title: title, Body: m.Text, Agent: agent, RepoKey: repo, Readers: msgReaders(m, dels),
	}, true
}

// fromAgent reports whether an agent (not the person or Irori) sent m.
func fromAgent(m *Message) bool {
	return !m.From.IsUser() && m.From.Kind != "" && m.From.Kind != SystemSender().Kind
}

// msgReaders are the agents that may find a message: its agent sender and
// the agents of its visible deliveries (a held copy is no reader, as the
// policy keeps it from them). Never empty, so no other agent reads it; the
// person reads every message. x.mu is held.
func msgReaders(m *Message, dels []*drec) []string {
	var out []string
	add := func(k string) {
		if k != "" && !isUserKey(k) && !slices.Contains(out, k) {
			out = append(out, k)
		}
	}
	if fromAgent(m) {
		add(m.From.Key)
	}
	for _, d := range dels {
		if visible(d) {
			add(d.To)
		}
	}
	if len(out) == 0 {
		out = append(out, SystemKey) // only the person: no agent is ever SystemKey
	}
	return out
}

func (x *inbox) senderName(m *Message) string {
	if m.From.Name != "" {
		return m.From.Name
	}
	return m.From.Key
}

// recipientName is who a message went to: the repo for repo and PR
// messages, else the first delivery's agent by its known name.
func (x *inbox) recipientName(m *Message, dels []*drec) string {
	switch m.To.Kind {
	case ToRepo, ToPR:
		return m.To.Value
	}
	for _, d := range dels {
		if d.To != "" {
			if n := x.known[d.To]; n != "" {
				return n
			}
			return d.To
		}
	}
	return m.To.Value
}

func (b *Inbox) searchDocs(ctx context.Context, since time.Time, limit int) ([]search.Doc, error) {
	if limit <= 0 {
		return nil, nil
	}
	x := b.impl
	x.mu.Lock()
	byMsg := x.msgDels()
	var docs []search.Doc
	for id, m := range x.msgs {
		if d, ok := x.msgDoc(m, byMsg[id]); ok && !d.Updated.Before(since) {
			docs = append(docs, d)
		}
	}
	x.mu.Unlock()
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	slices.SortStableFunc(docs, func(a, b search.Doc) int {
		if !a.Updated.Equal(b.Updated) {
			return a.Updated.Compare(b.Updated)
		}
		return strings.Compare(a.Ref, b.Ref)
	})
	if len(docs) > limit {
		docs = docs[:limit]
	}
	return docs, nil
}

func (b *Inbox) searchShow(ref string, o search.ShowOptions) (search.Shown, error) {
	kind, id, err := search.ParseRef(ref)
	if err != nil {
		return search.Shown{}, err
	}
	if kind != search.KindMsg {
		return search.Shown{}, search.ErrNotFound
	}
	x := b.impl
	x.mu.Lock()
	defer x.mu.Unlock()
	m := x.msgs[id]
	if m == nil {
		return search.Shown{}, search.ErrNotFound
	}
	byMsg := x.msgDels()
	doc, ok := x.msgDoc(m, byMsg[id])
	if !ok || !search.CanRead(o.Viewer, doc.Readers) {
		// Still held (search never gets past the policy), or not the
		// viewer's: either way it isn't there for them.
		return search.Shown{}, search.ErrNotFound
	}
	// readable is a related message the viewer may also show.
	readable := func(r *Message) bool {
		d, ok := x.msgDoc(r, byMsg[r.ID])
		return ok && search.CanRead(o.Viewer, d.Readers)
	}
	var sb strings.Builder
	line := func(format string, args ...any) {
		fmt.Fprintf(&sb, format, args...)
		sb.WriteByte('\n')
	}
	line("message %s from %s (%s, %s)", doc.Ref, x.senderName(m), m.From.Key, m.From.Kind)
	line("to: %s %s", m.To.Kind, m.To.Value)
	line("trust: %s, sent %s", m.Trust, m.SentAt.UTC().Format(time.RFC3339))
	if m.Reason != "" {
		line("reason: %s", m.Reason)
	}
	if m.Ref != "" {
		line("about: %s", m.Ref)
	}
	if m.Notice {
		line("(a Board notice: no reply needed)")
	}
	for _, d := range byMsg[id] {
		if !visible(d) {
			continue
		}
		mode := d.Mode
		if mode == "" {
			mode = ModeWake
		}
		to := d.To
		if to == "" {
			to = "(no desk yet)"
		}
		line("- delivery %s to %s: %s (%s)", d.ID, to, d.State, mode)
	}
	sb.WriteString("\ntext:\n" + m.Text + "\n")

	sh := search.Shown{Ref: doc.Ref, Kind: search.KindMsg, Title: clipTitleRunes(doc.Title)}
	relate := func(ref string) {
		if ref != "" && ref != sh.Ref && !slices.Contains(sh.Related, ref) && len(sh.Related) < search.MaxRelated {
			sh.Related = append(sh.Related, ref)
		}
	}
	if m.ParentID != "" {
		if p := x.msgs[m.ParentID]; p != nil {
			if readable(p) {
				relate(search.Ref(search.KindMsg, p.ID))
			}
		}
	}
	var replies []*Message
	for _, r := range x.msgs {
		if r.ParentID == id {
			if readable(r) {
				replies = append(replies, r)
			}
		}
	}
	slices.SortFunc(replies, func(a, b *Message) int { return a.SentAt.Compare(b.SentAt) })
	for _, r := range replies {
		relate(search.Ref(search.KindMsg, r.ID))
	}
	if _, _, err := search.ParseRef(m.Ref); err == nil {
		relate(m.Ref)
	}
	sh.Text, sh.More, sh.Next = search.Page(sb.String(), o)
	return sh, nil
}

func clipTitleRunes(s string) string {
	s = strings.Join(strings.Fields(s), " ")
	if r := []rune(s); len(r) > search.MaxTitle {
		return string(r[:search.MaxTitle-1]) + "…"
	}
	return s
}

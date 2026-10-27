package coord

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/search"
)

// showComments is how many of a task's newest comments Show prints in
// full; older ones are listed as Related refs.
const showComments = 10

// The search.Source behind Coord.SearchDocs and SearchShow (board.go). It
// reads only the exported views (ListTasks, Comments, GetTask, GetComment),
// so it doesn't care how the fold keeps them.

func (c *Coord) searchDocs(ctx context.Context, since time.Time, limit int) ([]search.Doc, error) {
	if limit <= 0 {
		return nil, nil
	}
	var docs []search.Doc
	for _, t := range c.ListTasks(TaskFilter{}) {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		d := taskDoc(t)
		if !d.Updated.Before(since) {
			docs = append(docs, d)
		}
		for _, cm := range c.Comments(t.ID) {
			if !cm.At.Before(since) {
				docs = append(docs, commentDoc(cm, t))
			}
		}
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

func taskDoc(t Task) search.Doc {
	var b strings.Builder
	for _, s := range []string{t.Objective, t.DoneCriteria, t.Note, t.Reason} {
		if s != "" {
			b.WriteString(s)
			b.WriteByte('\n')
		}
	}
	upd := t.UpdatedAt
	if upd.IsZero() {
		upd = t.CreatedAt
	}
	return search.Doc{
		Ref: search.Ref(search.KindTask, t.ID), Kind: search.KindTask, At: t.CreatedAt, Updated: upd,
		Title: t.Title, Body: strings.TrimSpace(b.String()), Agent: t.Holder, RepoKey: t.RepoKey,
	}
}

func commentDoc(cm Comment, t Task) search.Doc {
	agent := ""
	if !strings.HasPrefix(cm.By, "user:") {
		agent = cm.By
	}
	return search.Doc{
		Ref: search.Ref(search.KindComment, cm.ID), Kind: search.KindComment, At: cm.At, Updated: cm.At,
		Title: "comment on " + t.Title, Body: cm.Text, Agent: agent, RepoKey: t.RepoKey,
	}
}

func (c *Coord) searchShow(ref string, o search.ShowOptions) (search.Shown, error) {
	kind, id, err := search.ParseRef(ref)
	if err != nil {
		return search.Shown{}, err
	}
	var b strings.Builder
	line := func(format string, args ...any) {
		fmt.Fprintf(&b, format, args...)
		b.WriteByte('\n')
	}
	sh := search.Shown{Ref: search.Ref(kind, id), Kind: kind}
	switch kind {
	case search.KindTask:
		t, ok := c.GetTask(id)
		if !ok {
			return search.Shown{}, search.ErrNotFound
		}
		sh.Title = t.Title
		line("task %s: %s", sh.Ref, t.Title)
		line("state: %s", t.State)
		if t.RepoKey != "" {
			line("repo: %s", t.RepoKey)
		}
		if len(t.Scope) > 0 {
			line("scope: %s", strings.Join(t.Scope, ", "))
		}
		holders := strings.Join(t.Holders, ", ")
		if holders == "" {
			holders = "nobody"
		}
		if t.Shared {
			holders += " (shared)"
		}
		line("holders: %s", holders)
		line("created by %s at %s, updated %s", t.CreatedBy, searchStamp(t.CreatedAt), searchStamp(t.UpdatedAt))
		if t.State == Blocked {
			by := ""
			if t.BlockedBy != nil {
				by = " by " + string(t.BlockedBy.Kind)
				if t.BlockedBy.Ref != "" {
					by += " " + t.BlockedBy.Ref
				}
			}
			line("blocked%s: %s", by, t.Reason)
		}
		searchSection(&b, "objective", t.Objective)
		searchSection(&b, "done when", t.DoneCriteria)
		searchSection(&b, "note", t.Note)
		if len(t.Links) > 0 {
			var lb strings.Builder
			for _, l := range t.Links {
				fmt.Fprintf(&lb, "- %s %s\n", l.Label, l.Ref)
			}
			searchSection(&b, "links", lb.String())
		}
		cms := c.Comments(id)
		slices.SortStableFunc(cms, func(a, b Comment) int { return b.At.Compare(a.At) }) // newest first
		if len(cms) > 0 {
			var cb strings.Builder
			for _, cm := range cms[:min(len(cms), showComments)] {
				fmt.Fprintf(&cb, "- %s %s at %s: %s\n", search.Ref(search.KindComment, cm.ID), cm.By, searchStamp(cm.At), strings.Join(strings.Fields(cm.Text), " "))
			}
			if n := len(cms) - showComments; n > 0 {
				fmt.Fprintf(&cb, "... %d older comments (in related)\n", n)
			}
			searchSection(&b, "comments", cb.String())
			for _, cm := range cms[min(len(cms), showComments):] {
				if len(sh.Related) == search.MaxRelated {
					break
				}
				sh.Related = append(sh.Related, search.Ref(search.KindComment, cm.ID))
			}
		}
	case search.KindComment:
		cm, ok := c.GetComment(id)
		if !ok {
			return search.Shown{}, search.ErrNotFound
		}
		title := cm.TaskID
		if t, ok := c.GetTask(cm.TaskID); ok {
			title = t.Title
		}
		sh.Title = "comment on " + title
		line("comment %s on %s by %s at %s", sh.Ref, search.Ref(search.KindTask, cm.TaskID), cm.By, searchStamp(cm.At))
		if cm.Wake {
			line("(asked to wake the holders)")
		}
		searchSection(&b, "text", cm.Text)
		sh.Related = []string{search.Ref(search.KindTask, cm.TaskID)}
	default:
		return search.Shown{}, search.ErrNotFound
	}
	sh.Title = searchClipTitle(sh.Title)
	sh.Text, sh.More, sh.Next = search.Page(b.String(), o)
	return sh, nil
}

func searchSection(b *strings.Builder, label, body string) {
	if body = strings.TrimSpace(body); body != "" {
		b.WriteString("\n" + label + ":\n" + body + "\n")
	}
}

func searchStamp(t time.Time) string {
	if t.IsZero() {
		return "-"
	}
	return t.UTC().Format(time.RFC3339)
}

func searchClipTitle(s string) string {
	s = strings.Join(strings.Fields(s), " ")
	if r := []rune(s); len(r) > search.MaxTitle {
		return string(r[:search.MaxTitle-1]) + "…"
	}
	return s
}

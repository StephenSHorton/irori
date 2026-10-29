package coord

import (
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/search"
)

// search.Source for task: and comment: refs (contract in board.go).

const (
	defaultDocPage = 500 // SearchDocs limit when none is given
	showComments   = 10  // newest comments a task's Show renders
	showCommentCut = 600 // runes of each comment in a task's Show
)

func (c *coord) searchDocs(since time.Time, limit int) []search.Doc {
	if limit <= 0 {
		limit = defaultDocPage
	}
	c.mu.Lock()
	defer c.unlock()
	var out []search.Doc
	for _, t := range c.tasks {
		if t.UpdatedAt.Before(since) && !c.movedSince(t.ID, since) {
			continue
		}
		out = append(out, search.Doc{
			Ref: search.Ref(search.KindTask, t.ID), Kind: search.KindTask,
			At: t.CreatedAt, Updated: t.UpdatedAt, Title: t.Title,
			Body:  joinNonEmpty("\n", t.ShortRef(), t.Objective, t.DoneCriteria, t.Note, t.Reason, cardText(t)),
			Agent: t.Holder, RepoKey: t.RepoKey,
		})
	}
	for _, cm := range c.comments {
		if cm.At.Before(since) {
			continue
		}
		t := c.tasks[cm.TaskID]
		d := search.Doc{
			Ref: search.Ref(search.KindComment, cm.ID), Kind: search.KindComment,
			At: cm.At, Updated: cm.At, Title: "comment on " + t.Title, Body: cm.Text,
			RepoKey: t.RepoKey,
		}
		if !c.isOwner(cm.By) {
			d.Agent = cm.By
		}
		out = append(out, d)
	}
	sort.Slice(out, func(i, j int) bool {
		if !out[i].Updated.Equal(out[j].Updated) {
			return out[i].Updated.Before(out[j].Updated)
		}
		return out[i].Ref < out[j].Ref
	})
	if len(out) > limit {
		out = out[:limit:limit]
	}
	return out
}

// cardText is what search finds of a task's steps and open blocker items.
func cardText(t *Task) string {
	var parts []string
	for _, s := range t.Steps {
		parts = append(parts, s.Text)
	}
	for _, b := range t.Blockers {
		if b.Open() && b.Question != t.Reason {
			parts = append(parts, b.Question)
		}
		if b.Open() && b.Context != "" {
			parts = append(parts, b.Context)
		}
	}
	return strings.Join(parts, "\n")
}

func joinNonEmpty(sep string, parts ...string) string {
	var keep []string
	for _, p := range parts {
		if p != "" {
			keep = append(keep, p)
		}
	}
	return strings.Join(keep, sep)
}

func (c *coord) searchShow(ref string, o search.ShowOptions) (search.Shown, error) {
	kind, id, err := search.ParseRef(ref)
	if err != nil {
		return search.Shown{}, err
	}
	c.mu.Lock()
	defer c.unlock()
	var sh search.Shown
	var full string
	switch kind {
	case search.KindTask:
		t := c.tasks[id]
		if short, ok := ParseShort(id); ok && t == nil {
			if t = c.tasks[c.shortIdx[short]]; t != nil {
				id = t.ID
			} else if err := c.ambiguous(short); err != nil {
				return search.Shown{}, err
			}
		}
		if t == nil {
			return search.Shown{}, fmt.Errorf("%w: %s", search.ErrNotFound, ref)
		}
		full, sh.Related = c.renderTask(t)
		sh.Title = cut(t.Title, search.MaxTitle)
	case search.KindComment:
		cm := c.comments[id]
		if cm == nil {
			return search.Shown{}, fmt.Errorf("%w: %s", search.ErrNotFound, ref)
		}
		t := c.tasks[cm.TaskID]
		var b strings.Builder
		fmt.Fprintf(&b, "Comment by %s on %q (%s), %s", cm.By, t.Title, t.Refs(), cm.At.Format(time.RFC3339))
		if cm.Wake {
			b.WriteString(", wake")
		}
		b.WriteString("\n\n")
		b.WriteString(cm.Text)
		b.WriteString("\n")
		for _, f := range cm.Attachments {
			b.WriteString("attached: " + f.Describe() + " (fetch it with attachment_get)\n")
		}
		full = b.String()
		sh.Title = cut("comment on "+t.Title, search.MaxTitle)
		sh.Related = []string{search.Ref(search.KindTask, t.ID)}
	default:
		return search.Shown{}, fmt.Errorf("%w: coord has no %s refs", search.ErrNotFound, kind)
	}
	sh.Ref, sh.Kind = search.Ref(kind, id), kind
	sh.Text, sh.More, sh.Next = search.Page(full, o)
	return sh, nil
}

// renderTask is a task's Show text and Related refs. The caller holds c.mu.
func (c *coord) renderTask(t *Task) (string, []string) {
	var b strings.Builder
	line := func(label, v string) {
		if v != "" {
			fmt.Fprintf(&b, "%s: %s\n", label, v)
		}
	}
	fmt.Fprintf(&b, "Task %s %q (task:%s)\n", t.ShortRef(), t.Title, t.ID)
	if t.ShortWas != "" {
		fmt.Fprintf(&b, "renumbered: it was #%s, until cards made on devices that were apart turned out to have #%s too; #%s alone now names none of them\n", t.ShortWas, t.ShortWas, t.ShortWas)
	}
	state := string(t.State)
	if t.Shared {
		state += ", shared"
	}
	line("state", state)
	line("repo", t.RepoKey)
	line("holders", strings.Join(t.Holders, ", "))
	line("assigned, not claimed yet", strings.Join(t.Assigned, ", "))
	line("created", t.CreatedBy+" at "+t.CreatedAt.Format(time.RFC3339))
	line("updated", t.UpdatedAt.Format(time.RFC3339))
	if t.State == Blocked {
		bl := t.Reason
		if bb := t.BlockedBy; bb != nil {
			on := string(bb.Kind)
			if bb.Ref != "" {
				on += " " + bb.Ref
			}
			bl += " (on " + on + ")"
		}
		line("blocked", bl)
	}
	line("watching", strings.Join(t.Watchers, ", "))
	line("scope", strings.Join(t.Scope, ", "))
	line("note", t.Note)
	if len(t.Steps) > 0 {
		d, n := t.Progress()
		fmt.Fprintf(&b, "\nsteps (%d/%d done):\n", d, n)
		for _, s := range t.Steps {
			mark := "[ ]"
			switch {
			case s.Done:
				mark = "[x]"
			case s.Doing:
				mark = "[>]"
			}
			fmt.Fprintf(&b, "- %s %s %s\n", mark, s.ID, s.Text)
		}
	}
	if len(t.Blockers) > 0 {
		b.WriteString("\nblockers:\n")
		for _, bl := range t.Blockers {
			state := "open"
			if r := bl.Resolved; r != nil {
				state = "answered by " + r.By + ": " + r.Answer
				if r.Closed {
					state = "closed by " + r.By
					if r.Answer != "" {
						state += ": " + r.Answer
					}
				}
			}
			fmt.Fprintf(&b, "- %s (%s) %s, raised by %s at %s\n", bl.ID, state, bl.Question, bl.RaisedBy, bl.At.Format(time.RFC3339))
			if bl.Context != "" {
				fmt.Fprintf(&b, "  context: %s\n", bl.Context)
			}
			for i, o := range bl.Options {
				if o.Recommended {
					fmt.Fprintf(&b, "  option %d: %s (recommended by %s)", i+1, o.Answer, o.By)
				} else {
					fmt.Fprintf(&b, "  option %d: %s (from %s)", i+1, o.Answer, o.By)
				}
				if o.Why != "" {
					fmt.Fprintf(&b, ": %s", o.Why)
				}
				b.WriteString("\n")
			}
		}
	}
	if t.Objective != "" {
		fmt.Fprintf(&b, "\nobjective:\n%s\n", t.Objective)
	}
	if t.DoneCriteria != "" {
		fmt.Fprintf(&b, "\ndone when:\n%s\n", t.DoneCriteria)
	}
	if len(t.Links) > 0 {
		b.WriteString("\nlinks:\n")
		for _, l := range t.Links {
			fmt.Fprintf(&b, "- %s: %s\n", l.Label, l.Ref)
		}
	}
	var related []string
	if bb := t.BlockedBy; t.State == Blocked && bb != nil && bb.Kind == BlockedOnTask {
		related = append(related, search.Ref(search.KindTask, bb.Ref))
	}
	ids := c.byTask[t.ID]
	if len(ids) > 0 {
		shown := ids[max(0, len(ids)-showComments):]
		older := ids[:len(ids)-len(shown)]
		fmt.Fprintf(&b, "\ncomments (%d", len(ids))
		if len(older) > 0 {
			fmt.Fprintf(&b, ", newest %d shown; Related has older ones", len(shown))
		}
		b.WriteString("):\n")
		for _, id := range shown {
			cm := c.comments[id]
			text := cut(cm.Text, showCommentCut)
			if text != cm.Text {
				text += " (show comment:" + cm.ID + " for all of it)"
			}
			if cm.Held {
				text = "(not let through the wake policy yet; text not on the card)"
			}
			if n := len(cm.Attachments); n > 0 {
				text += fmt.Sprintf(" [%d attached: show comment:%s]", n, cm.ID)
			}
			if cm.Files > 0 {
				text += fmt.Sprintf(" [%d files on msg:%s]", cm.Files, cm.MsgID)
			}
			who := cm.By
			if cm.MsgID != "" {
				// A message about the card: who sent it to whom.
				who += " -> " + cm.To
			}
			if cm.Gone {
				text += " (recipient gone, left on the card)"
			}
			fmt.Fprintf(&b, "- %s %s: %s\n", cm.At.Format(time.RFC3339), who, text)
		}
		for i := len(older) - 1; i >= 0 && len(related) < search.MaxRelated; i-- {
			related = append(related, search.Ref(search.KindComment, older[i]))
		}
	}
	return b.String(), related
}

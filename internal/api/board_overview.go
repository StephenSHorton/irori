package api

import (
	"fmt"
	"slices"

	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/search"
)

// overviewInput gathers what capture.Overview copies in from coord and the
// inbox for agent key (capture imports neither). Lists follow the
// OverviewInput field docs: inbox lines are the agent's newest open
// deliveries; blockers are blocked tasks in its repo; tasks are the other
// open ones there, its own first; Current is each holder's last-changed
// task.
func overviewInput(co *coord.Coord, ib *inbox.Inbox, cards []presence.AgentCard, key string) capture.OverviewInput {
	in := capture.OverviewInput{}
	for _, c := range cards {
		if c.Key == key {
			in.RepoKey = c.RepoKey
			break
		}
	}

	open := ib.For(key, false, countLimit) // newest first
	in.InboxOpen = len(open)
	for _, d := range open[:min(len(open), capture.OverviewLines)] {
		text := d.Message.From.Name + ": " + d.Message.Text
		if d.Message.From.Name == "" {
			text = d.Message.Text
		}
		in.Inbox = append(in.Inbox, capture.Line{Ref: search.Ref(search.KindMsg, d.MessageID), Text: text, At: d.CreatedAt})
	}
	for _, d := range ib.List(countLimit) {
		if d.To == key && d.State == inbox.Held {
			in.InboxHeld++
		}
	}

	tasks := co.ListTasks(coord.TaskFilter{Open: true})
	in.BoardCount = len(tasks)
	// Newest change first, so Current keeps each holder's latest task and
	// the lists read newest first.
	slices.SortStableFunc(tasks, func(a, b coord.Task) int { return b.UpdatedAt.Compare(a.UpdatedAt) })
	in.Current = map[string]capture.Line{}
	var mine, others []capture.Line
	for _, t := range tasks {
		line := capture.Line{Ref: search.Ref(search.KindTask, t.ID), Text: t.Title, At: t.UpdatedAt}
		for _, h := range t.Holders {
			if _, ok := in.Current[h]; !ok && !slices.Contains(t.Assigned, h) {
				in.Current[h] = line
			}
		}
		if in.RepoKey != "" && t.RepoKey != in.RepoKey {
			continue
		}
		if t.State == coord.Blocked {
			if len(in.Blockers) < capture.OverviewLines {
				line.Text = fmt.Sprintf("%s (blocked: %s)", t.Title, t.Reason)
				in.Blockers = append(in.Blockers, line)
			}
			continue
		}
		if slices.Contains(t.Holders, key) {
			mine = append(mine, line)
		} else {
			others = append(others, line)
		}
	}
	// capture caps Tasks and counts the rest into MoreTasks.
	in.Tasks = append(mine, others...)
	return in
}

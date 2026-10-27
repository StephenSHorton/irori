package tools

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/search"
)

// Progressive disclosure (docs/design/board.md "Contracts"): overview, then
// short lists with refs (search, commits, prs, board), then show(ref). The
// caller's key is always the Viewer, never an argument, so a message stays
// with its sender and recipients.

const (
	defaultCommitLimit = 10
	maxCommitLimit     = 50
	maxPRs             = 30
	overviewInboxScan  = 500 // deliveries scanned for the held count
)

// viewer is who search and show read as: the calling agent, always.
func viewer(c Caller) string { return c.Sender.Key }

// --- overview ---

// current maps each agent to the open task it holds (and has taken up) that
// changed last: capture's "doing" falls back to it.
func current(ts []coord.Task) map[string]capture.Line {
	out := map[string]capture.Line{}
	at := map[string]time.Time{}
	for _, t := range ts {
		for _, h := range t.Holders {
			if slices.Contains(t.Assigned, h) {
				continue // assigned, not taken up: not what it is doing
			}
			if prev, ok := at[h]; ok && !t.UpdatedAt.After(prev) {
				continue
			}
			at[h] = t.UpdatedAt
			out[h] = capture.Line{Ref: search.Ref(search.KindTask, t.ID), Text: "on the Board: " + oneLine(t.Title, maxCardTitle), At: t.UpdatedAt}
		}
	}
	return out
}

func taskOverviewLine(t coord.Task, n names) capture.Line {
	text := "[" + string(t.State) + "] " + oneLine(t.Title, maxCardTitle)
	switch {
	case t.State == coord.Blocked:
		text += ": " + oneLine(t.Reason, 80)
	case len(t.Holders) > 0:
		text += ", " + n.who(t.Holders[0])
	}
	return capture.Line{Ref: search.Ref(search.KindTask, t.ID), Text: text, At: t.UpdatedAt}
}

// overviewInput gathers what capture's Overview doesn't own: the caller's
// inbox and the Board in its repo.
func (s *set) overviewInput(c Caller, n names) capture.OverviewInput {
	key := c.Sender.Key
	repo := n[key].RepoKey
	ib, co := s.b.Inbox(), s.b.Coord()
	in := capture.OverviewInput{RepoKey: repo}

	open := ib.For(key, false, inbox.DefaultInboxList)
	in.InboxOpen = len(open)
	for _, d := range open {
		if len(in.Inbox) == capture.OverviewLines {
			break
		}
		text := "from " + n.sender(d.Message.From) + ": "
		if r := strings.TrimSpace(d.Message.Reason); r != "" {
			text += oneLine(r, 80)
		} else {
			text += oneLine(d.Message.Text, 80)
		}
		in.Inbox = append(in.Inbox, capture.Line{Ref: search.Ref(search.KindMsg, d.Message.ID), Text: text, At: d.CreatedAt})
	}
	for _, d := range ib.List(overviewInboxScan) {
		if d.To == key && d.State == inbox.Held {
			in.InboxHeld++
		}
	}

	all := co.ListTasks(coord.TaskFilter{Open: true})
	in.BoardCount = len(all)
	in.Current = current(all)
	v := co.Board(coord.BoardFilter{RepoKey: repo, For: key, DoneLimit: 1})
	var tasks []capture.Line
	for _, cd := range v.NeedsYou {
		if cd.State == coord.Blocked {
			in.Blockers = append(in.Blockers, taskOverviewLine(cd.Task, n))
		} else {
			tasks = append(tasks, taskOverviewLine(cd.Task, n))
		}
	}
	for _, cd := range v.Blocked {
		in.Blockers = append(in.Blockers, taskOverviewLine(cd.Task, n))
	}
	var mine, others []capture.Line
	for _, cd := range append(slices.Clone(v.Working), v.Open...) {
		if slices.Contains(cd.Holders, key) {
			mine = append(mine, taskOverviewLine(cd.Task, n))
		} else {
			others = append(others, taskOverviewLine(cd.Task, n))
		}
	}
	tasks = append(append(tasks, mine...), others...)
	if len(in.Blockers) > capture.OverviewLines {
		in.Blockers = in.Blockers[:capture.OverviewLines]
	}
	if len(tasks) > capture.OverviewLines {
		in.MoreTasks = len(tasks) - capture.OverviewLines
		tasks = tasks[:capture.OverviewLines]
	}
	in.Tasks = tasks
	return in
}

func writeLines(b *strings.Builder, head string, ls []capture.Line, now time.Time) {
	if len(ls) == 0 {
		return
	}
	b.WriteString(head + ":\n")
	for _, l := range ls {
		fmt.Fprintf(b, "- %s %s", oneLine(l.Ref, 200), oneLine(l.Text, capture.OverviewLineMax))
		if !l.At.IsZero() {
			b.WriteString(" (" + ago(now, l.At) + ")")
		}
		b.WriteString("\n")
	}
}

func (s *set) overview(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a struct{}
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	cp := s.b.Capture()
	if cp == nil {
		return captureOff()
	}
	n := s.roster()
	now := s.now()
	ov := cp.Overview(ctx, c.Sender.Key, s.overviewInput(c, n))
	var b strings.Builder
	where := "no repo"
	if ov.RepoKey != "" {
		where = "repo " + oneLine(ov.RepoKey, 160)
	}
	fmt.Fprintf(&b, "Overview for %s, %s. %s\n", n.who(c.Sender.Key), where, capturedUntrusted)
	fmt.Fprintf(&b, "\nAgents live (%d):\n", len(ov.Agents)+ov.MoreAgents)
	for _, ag := range ov.Agents {
		you := ""
		if ag.Key == c.Sender.Key {
			you = " (you)"
		}
		fmt.Fprintf(&b, "- %s%s [%s], key %s", oneLine(ag.Name, 80), you, oneLine(ag.Status, 30), oneLine(ag.Key, 120))
		if ag.RepoKey != "" && ag.RepoKey != ov.RepoKey {
			b.WriteString(", repo " + oneLine(ag.RepoKey, 160))
		}
		if d := oneLine(ag.Doing.Text, capture.OverviewLineMax); d != "" {
			fmt.Fprintf(&b, ": %s (%s)", quoted(d, capture.OverviewLineMax), oneLine(ag.Doing.Ref, 200))
		} else if ag.Doing.Ref != "" {
			fmt.Fprintf(&b, " (%s)", oneLine(ag.Doing.Ref, 200))
		}
		b.WriteString("\n")
	}
	if ov.MoreAgents > 0 {
		fmt.Fprintf(&b, "(%d more: the agents tool lists them all)\n", ov.MoreAgents)
	}
	fmt.Fprintf(&b, "\nInbox: %d open", ov.InboxOpen)
	if ov.InboxHeld > 0 {
		fmt.Fprintf(&b, ", %d held for the person to release", ov.InboxHeld)
	}
	b.WriteString(".\n")
	writeLines(&b, "Newest", ov.Inbox, now)
	if len(ov.Blockers) > 0 || len(ov.Tasks) > 0 {
		b.WriteString("\n")
	}
	writeLines(&b, "Blocked", ov.Blockers, now)
	writeLines(&b, "On the Board", ov.Tasks, now)
	if ov.MoreTasks > 0 {
		fmt.Fprintf(&b, "(%d more cards: the board tool lists them)\n", ov.MoreTasks)
	}
	if len(ov.Commits) > 0 || len(ov.PRs) > 0 {
		b.WriteString("\n")
	}
	writeLines(&b, "Recent commits", ov.Commits, now)
	writeLines(&b, "Pull requests", ov.PRs, now)
	b.WriteString("\n" + oneLine(ov.Hint, 300) + ".\n")
	return b.String()
}

// --- search ---

type searchArgs struct {
	Query  string   `json:"query"`
	Kinds  []string `json:"kinds"`
	Repo   string   `json:"repo"`
	Agent  string   `json:"agent"`
	Offset int      `json:"offset"`
	Limit  int      `json:"limit"`
}

func (s *set) search(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a searchArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	cp := s.b.Capture()
	if cp == nil {
		return captureOff()
	}
	if a.Offset < 0 {
		return errText("offset is how many hits to skip: 0 or more.")
	}
	q := search.Query{Text: strings.TrimSpace(a.Query), RepoKey: strings.TrimSpace(a.Repo), Offset: a.Offset, Viewer: viewer(c)}
	for _, k := range a.Kinds {
		kind := search.Kind(strings.ToLower(strings.TrimSpace(k)))
		if kind == "" {
			continue
		}
		if kind == search.KindAgent || !slices.Contains(search.Kinds, kind) {
			return errText(fmt.Sprintf("kinds are turn, edit, commit, msg, task, comment, pr and decision, not %q.", oneLine(k, 40)))
		}
		q.Kinds = append(q.Kinds, kind)
	}
	if v := strings.TrimSpace(a.Agent); v != "" {
		k, err := s.resolveAgent(v)
		if err != nil {
			return mapErr(err)
		}
		q.Agent = k
	}
	limit := a.Limit
	if limit <= 0 {
		limit = search.DefaultLimit
	}
	limit = min(limit, search.MaxLimit)
	q.Limit = limit + 1
	hits, err := cp.Search(ctx, q)
	if err != nil {
		return errText("search failed: " + err.Error() + ". Use plain words.")
	}
	more := len(hits) > limit
	if more {
		hits = hits[:limit]
	}
	what := "newest items"
	if q.Text != "" {
		what = fmt.Sprintf("hits for %q", oneLine(q.Text, 120))
	}
	if len(hits) == 0 {
		if a.Offset > 0 {
			return fmt.Sprintf("No more %s past offset %d.", what, a.Offset)
		}
		return fmt.Sprintf("No %s. Try fewer or other words, or drop kinds, repo or agent.", what)
	}
	n := s.roster()
	now := s.now()
	var b strings.Builder
	fmt.Fprintf(&b, "%d %s", len(hits), what)
	if a.Offset > 0 {
		fmt.Fprintf(&b, " (from offset %d)", a.Offset)
	}
	b.WriteString(". " + capturedUntrusted + "\n")
	budget := MaxResult - 200
	shown := 0
	for _, h := range hits {
		var line strings.Builder
		fmt.Fprintf(&line, "- %s %s %s", oneLine(h.Ref, 200), short2(now, h.At), quoted(h.Title, search.MaxTitle))
		if h.Agent != "" {
			line.WriteString(", " + n.who(h.Agent))
		}
		line.WriteString("\n")
		if sn := strings.TrimSpace(h.Snippet); sn != "" && sn != strings.TrimSpace(h.Title) {
			fmt.Fprintf(&line, "  %s\n", quoted(sn, search.MaxSnippet))
		}
		if b.Len()+line.Len() > budget {
			more = true
			break
		}
		b.WriteString(line.String())
		shown++
	}
	if more {
		fmt.Fprintf(&b, "(more: offset=%d; search again with the same arguments and offset = that value)\n", a.Offset+shown)
	}
	b.WriteString("show <ref> for any hit.\n")
	return b.String()
}

// short2 is a hit's time: the date and clock, and how long ago.
func short2(now, t time.Time) string {
	if t.IsZero() {
		return "(no time)"
	}
	return t.In(now.Location()).Format("2006-01-02 15:04") + " (" + ago(now, t) + ")"
}

// --- show ---

type showArgs struct {
	Ref    string `json:"ref"`
	Offset int    `json:"offset"`
}

func (s *set) show(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a showArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	cp := s.b.Capture()
	if cp == nil {
		return captureOff()
	}
	ref := strings.TrimSpace(a.Ref)
	if ref == "" {
		return errText("give ref, exactly as a list line printed it (task:…, msg:…, turn:…, commit:…).")
	}
	if a.Offset < 0 {
		return errText("offset is where to continue: 0 or more.")
	}
	sh, err := cp.Show(ctx, ref, search.ShowOptions{Offset: a.Offset, Viewer: viewer(c)})
	if err != nil {
		if errors.Is(err, search.ErrNotFound) || errors.Is(err, capture.ErrNotFound) {
			return errText(fmt.Sprintf("nothing you may read has the ref %q. search finds refs; a message shows only to its sender and recipients.", oneLine(ref, 200)))
		}
		return mapErr(err)
	}
	var b strings.Builder
	fmt.Fprintf(&b, "%s %s", oneLine(sh.Ref, 200), quoted(sh.Title, search.MaxTitle))
	if a.Offset > 0 {
		fmt.Fprintf(&b, " (from offset %d)", a.Offset)
	}
	b.WriteString(". " + capturedUntrusted + "\n\n")
	b.WriteString(strings.TrimRight(sh.Text, "\n") + "\n")
	if sh.More {
		fmt.Fprintf(&b, "\n(more: offset=%d; show ref = %s with offset = that value continues)\n", sh.Next, oneLine(sh.Ref, 200))
	}
	if len(sh.Related) > 0 {
		var rel []string
		for _, r := range sh.Related {
			rel = append(rel, oneLine(r, 200))
		}
		b.WriteString("Related: " + strings.Join(rel, ", ") + "\n")
	}
	return b.String()
}

// --- commits / prs ---

type commitsArgs struct {
	Repo  string `json:"repo"`
	Path  string `json:"path"`
	Limit int    `json:"limit"`
}

func (s *set) commits(_ context.Context, c Caller, raw json.RawMessage) string {
	var a commitsArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	cp := s.b.Capture()
	if cp == nil {
		return captureOff()
	}
	repo := strings.TrimSpace(a.Repo)
	if repo == "" {
		repo = s.callerRepo(c)
	}
	if repo == "" {
		return errText("give repo: a repoKey from the agents tool (Irori knows no repo for you).")
	}
	limit := a.Limit
	if limit <= 0 {
		limit = defaultCommitLimit
	}
	limit = min(limit, maxCommitLimit)
	path := strings.Trim(strings.ReplaceAll(strings.TrimSpace(a.Path), `\`, "/"), "/")
	path = strings.TrimPrefix(path, "./")
	scan := limit + 1
	if path != "" {
		scan = 500
	}
	var list []capture.CommitView
	for _, cv := range cp.Commits(repo, scan) {
		if path == "" || touches(cv, path) {
			list = append(list, cv)
		}
	}
	where := oneLine(repo, 160)
	if path != "" {
		where += ", touching " + oneLine(path, 200)
	}
	if len(list) == 0 {
		return "No commits seen in " + where + ". Irori sees commits made in worktrees it watches."
	}
	more := len(list) > limit
	if more {
		list = list[:limit]
	}
	n := s.roster()
	now := s.now()
	var b strings.Builder
	fmt.Fprintf(&b, "%d commits in %s, newest first. %s\n", len(list), where, capturedUntrusted)
	budget := MaxResult - 240
	cut := 0
	for i, cv := range list {
		var line strings.Builder
		fmt.Fprintf(&line, "- commit:%s %s %s", oneLine(cv.SHA, 64), short2(now, cv.At), quoted(cv.Subject, 160))
		if br := oneLine(cv.Branch, 80); br != "" {
			line.WriteString(" on " + br)
		}
		if au := oneLine(cv.Author, 80); au != "" {
			line.WriteString(", author " + au)
		}
		if cv.Link != nil && cv.Link.Committer != nil {
			line.WriteString(", committed by " + n.who(cv.Link.Committer.Key))
		}
		line.WriteString("\n")
		if b.Len()+line.Len() > budget {
			cut = len(list) - i
			break
		}
		b.WriteString(line.String())
	}
	switch {
	case cut > 0:
		fmt.Fprintf(&b, "(%d more not shown; narrow with path, or search kinds=[\"commit\"] with words from the subject)\n", cut)
	case more:
		fmt.Fprintf(&b, "(more: raise limit, up to %d; or search kinds=[\"commit\"] with words from the subject)\n", maxCommitLimit)
	}
	b.WriteString("show commit:<sha> for its files and the turns behind it.\n")
	return b.String()
}

// touches: a commit changed path, or something under it.
func touches(cv capture.CommitView, path string) bool {
	for _, f := range cv.Files {
		if f.Path == path || strings.HasPrefix(f.Path, path+"/") {
			return true
		}
	}
	return false
}

func (s *set) prs(_ context.Context, c Caller, raw json.RawMessage) string {
	var a struct {
		Repo string `json:"repo"`
	}
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	cp := s.b.Capture()
	if cp == nil {
		return captureOff()
	}
	repo := strings.TrimSpace(a.Repo)
	if repo == "" {
		repo = s.callerRepo(c)
	}
	gh := repo
	if strings.Contains(repo, ":") {
		gh = ledger.RepoFromKey(repo)
	}
	if gh == "" || !strings.Contains(gh, "/") {
		return errText("give repo: a GitHub repo as remote:github.com/<owner>/<name> or <owner>/<name>.")
	}
	// ForRepo is newest PR number first, then best owner first.
	var keys []string
	byKey := map[string][]ledger.Entry{}
	for _, e := range cp.PRs(gh) {
		if _, ok := byKey[e.Key]; !ok {
			keys = append(keys, e.Key)
		}
		byKey[e.Key] = append(byKey[e.Key], e)
	}
	if len(keys) == 0 {
		return "No pull requests linked to agents in " + oneLine(gh, 160) + "."
	}
	more := len(keys) > maxPRs
	if more {
		keys = keys[:maxPRs]
	}
	n := s.roster()
	var b strings.Builder
	fmt.Fprintf(&b, "%d pull requests in %s, newest first. %s\n", len(keys), oneLine(gh, 160), untrusted)
	for _, k := range keys {
		es := byKey[k]
		var who []string
		for _, e := range es {
			who = append(who, oneLine(string(e.Role), 20)+" "+n.who(e.Session))
		}
		fmt.Fprintf(&b, "- %s: %s", oneLine(k, 200), strings.Join(who, ", "))
		for _, e := range es {
			if u := oneLine(e.URL, 200); u != "" {
				b.WriteString(" " + u)
				break
			}
		}
		b.WriteString("\n")
	}
	if more {
		b.WriteString("(more not shown: search kinds=[\"pr\"] finds older ones)\n")
	}
	b.WriteString("show pr:<owner>/<name>#<n> for one.\n")
	return b.String()
}

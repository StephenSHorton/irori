package tools

import (
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
)

const maxTaskList = 50

type taskCreateArgs struct {
	Title        string   `json:"title"`
	Objective    string   `json:"objective"`
	DoneCriteria string   `json:"done_criteria"`
	Repo         string   `json:"repo"`
	Scope        []string `json:"scope"`
}

func (s *set) taskCreate(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a taskCreateArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	if strings.TrimSpace(a.Title) == "" {
		return errText("give the task a title.")
	}
	if strings.TrimSpace(a.Objective) == "" {
		return errText("give the task an objective: what needs doing and why.")
	}
	repo := strings.TrimSpace(a.Repo)
	if repo == "" {
		repo = s.roster()[c.Sender.Key].RepoKey
	}
	var scope []string
	for _, g := range a.Scope {
		if g = strings.TrimSpace(g); g != "" {
			scope = append(scope, g)
		}
	}
	t, err := s.b.Coord().CreateTask(ctx, c.Sender.Key, coord.NewTask{
		Title: strings.TrimSpace(a.Title), Objective: strings.TrimSpace(a.Objective),
		DoneCriteria: strings.TrimSpace(a.DoneCriteria), RepoKey: repo, Scope: scope,
	})
	if err != nil {
		return mapErr(err)
	}
	return "Created task " + t.ID + " (" + string(t.State) + ", unassigned). Claim it with task_claim or hand it over with task_assign.\n" +
		s.taskLine(t, s.roster())
}

type taskListArgs struct {
	Repo string `json:"repo"`
	Mine bool   `json:"mine"`
	All  bool   `json:"all"`
}

func (s *set) taskList(_ context.Context, c Caller, raw json.RawMessage) string {
	var a taskListArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	f := coord.TaskFilter{RepoKey: strings.TrimSpace(a.Repo), Open: !a.All}
	if a.Mine {
		if strings.TrimSpace(c.Sender.Key) == "" {
			return errText("mine needs a bound agent; call join first (Grok) or attach the Irori mod (Claude Code).")
		}
		f.Holder = c.Sender.Key
	}
	ts := s.b.Coord().ListTasks(f)
	what := "open tasks"
	if a.All {
		what = "tasks"
	}
	if f.RepoKey != "" {
		what += " in " + oneLine(f.RepoKey, 160)
	}
	if a.Mine {
		what += " held by you"
	}
	if len(ts) == 0 {
		return "No " + what + "."
	}
	head := fmt.Sprintf("%d %s, oldest first", len(ts), what)
	if len(ts) > maxTaskList {
		head += fmt.Sprintf(" (showing the newest %d)", maxTaskList)
		ts = ts[len(ts)-maxTaskList:]
	}
	n := s.roster()
	var b strings.Builder
	b.WriteString(head + ". " + untrusted + "\n")
	for _, t := range ts {
		b.WriteString("\n" + s.taskLine(t, n))
	}
	return b.String()
}

// taskLine is one task, every untrusted field folded.
func (s *set) taskLine(t coord.Task, n names) string {
	var b strings.Builder
	holder := "unassigned"
	if t.Holder != "" {
		holder = "held by " + n.who(t.Holder)
	}
	fmt.Fprintf(&b, "- task %s [%s] %s; %s\n", oneLine(t.ID, 80), oneLine(string(t.State), 30), oneLine(t.Title, maxFieldRunes), holder)
	fmt.Fprintf(&b, "  objective: %s\n", oneLine(t.Objective, maxObjective))
	if d := oneLine(t.DoneCriteria, maxObjective); d != "" {
		fmt.Fprintf(&b, "  done when: %s\n", d)
	}
	if t.RepoKey != "" {
		fmt.Fprintf(&b, "  repoKey: %s\n", oneLine(t.RepoKey, 160))
	}
	if len(t.Scope) > 0 {
		fmt.Fprintf(&b, "  scope: %s\n", oneLine(strings.Join(t.Scope, ", "), maxFieldRunes))
	}
	if note := oneLine(t.Note, maxFieldRunes); note != "" {
		fmt.Fprintf(&b, "  note: %s\n", note)
	}
	fmt.Fprintf(&b, "  created by %s %s, updated %s\n", n.who(t.CreatedBy), ago(s.now(), t.CreatedAt), ago(s.now(), t.UpdatedAt))
	return b.String()
}

type taskIDArgs struct {
	TaskID string `json:"task_id"`
}

func (s *set) taskClaim(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a taskIDArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := strings.TrimSpace(a.TaskID)
	if id == "" {
		return errText("give task_id (see task_list).")
	}
	t, err := s.b.Coord().ClaimTask(ctx, c.Sender.Key, id)
	if err != nil {
		return mapErr(err)
	}
	return "Claimed task " + t.ID + "; you are its single writer. Set its state with task_status as you go.\n" + s.taskLine(t, s.roster())
}

type taskAssignArgs struct {
	TaskID string `json:"task_id"`
	To     string `json:"to"`
}

func (s *set) taskAssign(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a taskAssignArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := strings.TrimSpace(a.TaskID)
	if id == "" {
		return errText("give task_id (see task_list).")
	}
	n := s.roster()
	to, msg := s.resolve(n, strings.TrimSpace(a.To))
	if msg != "" {
		return msg
	}
	t, err := s.b.Coord().AssignTask(ctx, c.Sender.Key, id, to)
	if err != nil {
		return mapErr(err)
	}
	return "Assigned task " + t.ID + " to " + n.who(to) + ". They aren't woken by this; send them a message if it's urgent.\n" + s.taskLine(t, n)
}

// resolve finds one live agent by exact key, key or session prefix (8+
// characters), card name or alias. On failure the second value is the
// "Error: …" text.
func (s *set) resolve(n names, want string) (string, string) {
	if want == "" {
		return "", errText("give to: the agent's name or key (see the agents tool).")
	}
	lw := strings.ToLower(want)
	var keys []string
	for k := range n {
		if n.live(k) {
			keys = append(keys, k)
		}
	}
	slices.Sort(keys)
	for _, k := range keys {
		if strings.ToLower(k) == lw {
			return k, ""
		}
	}
	try := func(match func(k string) bool) (string, string, bool) {
		var m []string
		for _, k := range keys {
			if match(k) {
				m = append(m, k)
			}
		}
		switch len(m) {
		case 0:
			return "", "", false
		case 1:
			return m[0], "", true
		}
		who := make([]string, len(m))
		for i, k := range m {
			who[i] = n.who(k)
		}
		return "", errText(fmt.Sprintf("%q matches %d agents: %s. Use a key.", oneLine(want, 80), len(m), strings.Join(who, ", "))), true
	}
	if len(lw) >= inbox.MinKeyPrefixLen {
		if k, e, ok := try(func(k string) bool {
			lk := strings.ToLower(k)
			_, sess, _ := strings.Cut(lk, "/")
			return strings.HasPrefix(lk, lw) || strings.HasPrefix(sess, lw)
		}); ok {
			return k, e
		}
	}
	if k, e, ok := try(func(k string) bool { return strings.ToLower(strings.TrimSpace(n[k].Name)) == lw }); ok {
		return k, e
	}
	ib := s.b.Inbox()
	if k, e, ok := try(func(k string) bool { return slices.Contains(ib.Aliases(k), lw) }); ok {
		return k, e
	}
	return "", errText(fmt.Sprintf("no running agent by the name or key %q. Use the agents tool to see names and keys.", oneLine(want, 80)))
}

type taskStatusArgs struct {
	TaskID string `json:"task_id"`
	State  string `json:"state"`
	Note   string `json:"note"`
}

func (s *set) taskStatus(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a taskStatusArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := strings.TrimSpace(a.TaskID)
	if id == "" {
		return errText("give task_id (see task_list).")
	}
	st := coord.TaskState(strings.ToLower(strings.TrimSpace(a.State)))
	if st == "" {
		return errText("give state: submitted, working, input_required, completed, failed or canceled.")
	}
	t, err := s.b.Coord().SetTaskState(ctx, c.Sender.Key, id, st, strings.TrimSpace(a.Note))
	if err != nil {
		return mapErr(err)
	}
	return "Task " + t.ID + " is now " + string(t.State) + ".\n" + s.taskLine(t, s.roster())
}

type leaseArgs struct {
	Resource   string `json:"resource"`
	TTLMinutes *int   `json:"ttl_minutes"`
	Steal      bool   `json:"steal"`
}

func (s *set) lease(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a leaseArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	res := strings.TrimSpace(a.Resource)
	if res == "" {
		return errText("give resource: path:<repoKey>:<glob> or a name like gpu/qwen.")
	}
	var ttl time.Duration
	if a.TTLMinutes != nil {
		m := *a.TTLMinutes
		if m < 1 || m > int(coord.MaxTTL/time.Minute) {
			return errText(fmt.Sprintf("ttl_minutes must be 1 to %d, not %d.", int(coord.MaxTTL/time.Minute), m))
		}
		ttl = time.Duration(m) * time.Minute
	}
	l, err := s.b.Coord().Acquire(ctx, c.Sender.Key, res, ttl, a.Steal)
	if err != nil {
		if strings.HasPrefix(res, "path:") {
			if rk := s.roster()[c.Sender.Key].RepoKey; rk != "" {
				return mapErr(err) + " Your repoKey is " + oneLine(rk, 160) + "."
			}
		}
		return mapErr(err)
	}
	n := s.roster()
	out := "Leased " + oneLine(l.Resource, maxFieldRunes) + " until " + l.Until.Format(time.RFC3339) + " (" + in(s.now(), l.Until) + "). Lease again to renew; release when done."
	if l.StolenFrom != "" {
		out += " Taken from " + n.who(l.StolenFrom) + "; consider telling them."
	}
	return out
}

func (s *set) leases(_ context.Context, _ Caller, raw json.RawMessage) string {
	var a struct{}
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	ls := s.b.Coord().Leases()
	if len(ls) == 0 {
		return "No current leases."
	}
	n := s.roster()
	now := s.now()
	var b strings.Builder
	fmt.Fprintf(&b, "%d current leases. %s\n", len(ls), untrusted)
	for _, l := range ls {
		fmt.Fprintf(&b, "- %s held by %s, expires %s", oneLine(l.Resource, maxFieldRunes), n.who(l.Holder), in(now, l.Until))
		if l.StolenFrom != "" {
			fmt.Fprintf(&b, ", taken from %s", n.who(l.StolenFrom))
		}
		b.WriteString("\n")
	}
	return b.String()
}

func (s *set) release(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a struct {
		Resource string `json:"resource"`
	}
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	res := strings.TrimSpace(a.Resource)
	if res == "" {
		return errText("give resource, exactly as the leases tool shows it.")
	}
	if err := s.b.Coord().Release(ctx, c.Sender.Key, res); err != nil {
		return mapErr(err)
	}
	return "Released " + oneLine(res, maxFieldRunes) + "."
}

package tools

import (
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

const (
	defaultEventLimit = 20
	maxEventLimit     = 100
)

type agentsArgs struct {
	Status string `json:"status"`
	Cwd    string `json:"cwd"`
}

func (s *set) agents(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a agentsArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	cards := slices.Clone(s.b.Agents())
	slices.SortFunc(cards, func(x, y presence.AgentCard) int {
		if r := strings.Compare(strings.ToLower(x.Name), strings.ToLower(y.Name)); r != 0 {
			return r
		}
		return strings.Compare(x.Key, y.Key)
	})
	now := s.now()
	doing := s.doing(ctx, c)
	var b strings.Builder
	shown := 0
	for _, card := range cards {
		if !matchAgent(card, a) {
			continue
		}
		shown++
		writeAgent(&b, card, c.Sender.Key, now, doing[card.Key])
	}
	head := fmt.Sprintf("%d of %d agents on this device%s.", shown, len(cards), describeAgentFilter(a))
	if shown == 0 {
		return head
	}
	return head + " Message one with send (to = its name or key). " + untrusted + "\n" + b.String()
}

func matchAgent(c presence.AgentCard, a agentsArgs) bool {
	if st := strings.TrimSpace(a.Status); st != "" && !strings.EqualFold(c.Status, st) {
		return false
	}
	if cwd := strings.TrimSpace(a.Cwd); cwd != "" && !strings.Contains(normPath(c.Cwd), normPath(cwd)) {
		return false
	}
	return true
}

func normPath(p string) string { return strings.ToLower(strings.ReplaceAll(p, `\`, "/")) }

func describeAgentFilter(a agentsArgs) string {
	var parts []string
	if a.Cwd != "" {
		parts = append(parts, "cwd contains "+fmt.Sprintf("%q", oneLine(a.Cwd, 120)))
	}
	if a.Status != "" {
		parts = append(parts, "status "+oneLine(a.Status, 40))
	}
	if len(parts) == 0 {
		return ""
	}
	return " matching " + strings.Join(parts, ", ")
}

// doing is each live agent's one line from capture's overview (nil with
// capture off): its open turn's prompt, the card it holds, or its last reply.
func (s *set) doing(ctx context.Context, c Caller) map[string]capture.Line {
	cp := s.b.Capture()
	if cp == nil {
		return nil
	}
	ov := cp.Overview(ctx, c.Sender.Key, capture.OverviewInput{
		Current: current(s.b.Coord().ListTasks(coord.TaskFilter{Open: true})),
	})
	out := map[string]capture.Line{}
	for _, a := range ov.Agents {
		out[a.Key] = a.Doing
	}
	return out
}

func writeAgent(b *strings.Builder, c presence.AgentCard, self string, now time.Time, doing capture.Line) {
	// Every field here can come from a file a peer agent wrote, so all of
	// them go through oneLine, not only the free-text ones.
	key := oneLine(c.Key, 120)
	name := oneLine(c.Name, maxFieldRunes)
	if name == "" {
		name = "unnamed"
	}
	agent := oneLine(c.Agent, 60)
	if agent == "" {
		agent = "agent"
	}
	you := ""
	if c.Key != "" && c.Key == self {
		you = " (you)"
	}
	fmt.Fprintf(b, "\n- %s%s [%s] %s, key %s\n", name, you, oneLine(c.Status, 40), agent, key)
	if title := oneLine(c.Title, maxFieldRunes); title != "" {
		fmt.Fprintf(b, "  title: %s\n", title)
	}
	if c.Cwd != "" {
		fmt.Fprintf(b, "  cwd: %s\n", oneLine(c.Cwd, maxFieldRunes))
	}
	repo := oneLine(c.RepoKey, maxFieldRunes)
	if repo == "" {
		repo = "none (not in a git repo)"
	}
	fmt.Fprintf(b, "  repoKey: %s\n", repo)
	if d := oneLine(doing.Text, capture.OverviewLineMax); d != "" {
		fmt.Fprintf(b, "  doing: %s (%s)\n", quoted(d, capture.OverviewLineMax), oneLine(doing.Ref, 200))
	}
	mode := oneLine(c.PermissionMode, 60)
	switch {
	case c.Agent == "grok":
		mode = "n/a"
	case mode == "":
		mode = "unknown"
	}
	fmt.Fprintf(b, "  permissionMode: %s\n", mode)
	last := c.LastActivity
	if last.IsZero() {
		last = c.UpdatedAt
	}
	fmt.Fprintf(b, "  last activity: %s\n", ago(now, last))
	if txt := oneLine(c.LastText, maxFieldRunes); txt != "" {
		fmt.Fprintf(b, "  last text (untrusted): %q\n", txt)
	}
}

type eventsArgs struct {
	Kinds []string `json:"kinds"`
	Limit int      `json:"limit"`
}

func (s *set) events(ctx context.Context, _ Caller, raw json.RawMessage) string {
	var a eventsArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	limit := a.Limit
	if limit <= 0 {
		limit = defaultEventLimit
	}
	limit = min(limit, maxEventLimit)
	var kinds []string
	for _, k := range a.Kinds {
		if k = strings.TrimSpace(k); k != "" {
			kinds = append(kinds, k)
		}
	}
	evs, err := s.b.Events(ctx, store.Filter{Kinds: kinds, Limit: limit, Desc: true})
	if err != nil {
		return errText("Irori couldn't read its event log: " + err.Error())
	}
	if len(evs) > limit {
		evs = evs[:limit]
	}
	now := s.now()
	head := fmt.Sprintf("%d events on this device, newest first", len(evs))
	if len(kinds) > 0 {
		head += " (kinds: " + oneLine(strings.Join(kinds, ", "), 200) + ")"
	}
	if len(evs) == 0 {
		return head + "."
	}
	var b strings.Builder
	for _, ev := range evs {
		writeEvent(&b, ev, now)
	}
	return head + ". Message text is left out; the inbox tool shows messages addressed to you. " + untrusted + "\n" + b.String()
}

func writeEvent(b *strings.Builder, ev store.Event, now time.Time) {
	at := ev.HLC.Wall()
	when := "unknown time"
	if ev.HLC != 0 {
		when = at.In(now.Location()).Format("2006-01-02 15:04:05") + " (" + ago(now, at) + ")"
	}
	dev := oneLine(ev.DeviceID, maxFieldRunes)
	if r := []rune(dev); len(r) > 8 {
		dev = string(r[:8])
	}
	fmt.Fprintf(b, "- %s %s dev=%s seq=%d actor=%s %s\n", when, oneLine(ev.Kind, 60), dev, ev.Seq,
		oneLine(ev.Actor, 120), payloadText(redact(ev.Kind, ev.Payload)))
}

// Redact is a row as an agent may see it (the events tool, and the
// loopback API without the GUI's credential, v0.4 §6.8): message text and
// reasons are left out, and so is a turn's prompt when it is an Irori fence
// (it quotes the messages it delivered). The log holds every message, held
// ones included; only the recipient's own inbox shows text (Inbox.For).
func Redact(kind string, raw json.RawMessage) json.RawMessage { return redact(kind, raw) }

// redact is Redact.
func redact(kind string, raw json.RawMessage) json.RawMessage {
	var fields []string
	switch kind {
	case inbox.KindMessageSent:
		fields = []string{"text", "reason"}
	case capture.KindTurn:
		// The stored JSON may escape "<"; read the prompt itself.
		var t struct {
			Prompt string `json:"prompt"`
		}
		if json.Unmarshal(raw, &t) == nil && !strings.Contains(strings.ToLower(t.Prompt), "<irori-message") {
			return raw
		}
		fields = []string{"prompt"}
	default:
		return raw
	}
	var m map[string]any
	if err := json.Unmarshal(raw, &m); err != nil {
		return json.RawMessage(`{"text":"[left out]"}`)
	}
	for _, k := range fields {
		if _, ok := m[k]; ok {
			m[k] = "[left out]"
		}
	}
	out, err := json.Marshal(m)
	if err != nil {
		return json.RawMessage(`{"text":"[left out]"}`)
	}
	return out
}

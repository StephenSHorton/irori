package tools

import (
	"cmp"
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/inbox"
)

const (
	defaultInboxLimit = 20
	maxInboxLimit     = 50
)

type sendArgs struct {
	To     *string  `json:"to"`
	Repo   *string  `json:"repo"`
	Text   string   `json:"text"`
	Reason string   `json:"reason"`
	Attach []string `json:"attach"`
	TaskID string   `json:"task_id"`
}

func (s *set) send(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a sendArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	to, repo := trimPtr(a.To), trimPtr(a.Repo)
	var addr inbox.Address
	switch {
	case to != "" && repo != "":
		return errText("give exactly one of to (an agent) or repo (a repoKey), not both.")
	case isPRTarget(to):
		addr = inbox.Address{Kind: inbox.ToPR, Value: to}
	case to != "":
		addr = inbox.Address{Kind: inbox.ToAgent, Value: to}
	case repo != "":
		addr = inbox.Address{Kind: inbox.ToRepo, Value: repo}
	default:
		return errText("say who it's for: to = an agent's name or key (or a PR: pr:<owner>/<name>#<number> or its URL), or repo = a repoKey (see the agents tool).")
	}
	if strings.TrimSpace(a.Text) == "" {
		return errText("give the message text.")
	}
	// A message about a card names it as its Ref, and goes on its thread
	// once sent. An unknown card sends nothing.
	ref := ""
	if strings.TrimSpace(a.TaskID) != "" {
		id := taskID(a.TaskID)
		if _, ok := s.b.Coord().GetTask(id); !ok {
			return errText("no card task:" + oneLine(id, 80) + "; nothing was sent. The board tool shows the cards.")
		}
		ref = "task:" + id
	}
	files, msg := s.attachFiles(ctx, c, a.Attach)
	if msg != "" {
		return msg
	}
	o := inbox.SendOptions{Reason: strings.TrimSpace(a.Reason), Mode: inbox.ModeWake, Attachments: files, Ref: ref}
	m, d, err := s.b.Inbox().SendWith(ctx, c.Sender, addr, a.Text, o)
	if err != nil {
		return mapErr(err)
	}
	out := "Sent message " + m.ID + attachedLine(files) + " " + s.outcome(d)
	if ref != "" {
		out += s.recorded(ctx, m, cmp.Or(d.To, addr.Value), d.State == inbox.Held)
	}
	return out
}

// isPRTarget: to names a pull request ("pr:..." or a GitHub URL) rather
// than an agent. A malformed "pr:" value or github.com URL still counts (no
// agent name or key holds "github.com/"), so the inbox explains the PR
// forms instead of saying there's no agent by that name.
func isPRTarget(to string) bool {
	if len(to) >= 3 && strings.EqualFold(to[:3], "pr:") {
		return true
	}
	return strings.Contains(strings.ToLower(to), "github.com/")
}

type replyArgs struct {
	DeliveryID string `json:"delivery_id"`
	Text       string `json:"text"`
}

func (s *set) reply(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a replyArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := strings.TrimSpace(a.DeliveryID)
	if id == "" {
		return errText("give delivery_id: the delivery=\"…\" value of the <irori-message> you are answering.")
	}
	if strings.TrimSpace(a.Text) == "" {
		return errText("give the reply text.")
	}
	m, d, err := s.b.Inbox().Reply(ctx, c.Sender, id, a.Text)
	if err != nil {
		return mapErr(err)
	}
	// A reply keeps the Ref of the message it answers, so a reply about a
	// card goes on its thread too.
	return "Replied to delivery " + oneLine(id, 80) + " (now answered) with message " + m.ID + " " + s.outcome(d) +
		s.recorded(ctx, m, d.To, d.State == inbox.Held)
}

// outcome says where a just-sent message went and what happens next.
func (s *set) outcome(d inbox.Delivery) string {
	n := s.roster()
	var b strings.Builder
	if d.To != "" {
		b.WriteString("to " + n.who(d.To))
	} else {
		b.WriteString("to repo " + oneLine(d.RepoKey, 160))
	}
	pr := d.Message.To.Kind == inbox.ToPR
	if pr {
		b.WriteString(" about " + oneLine(d.Message.To.Value, 160))
	}
	b.WriteString(" (delivery " + d.ID + "): ")
	switch d.State {
	case inbox.Held:
		switch d.Hold {
		case inbox.HoldUnowned:
			b.WriteString("no agent in that repo can take it yet, so it waits for one to arrive or for the person to route it.")
		case inbox.HoldPaused:
			b.WriteString("held because the person paused all wakes; it goes out when they unpause. Don't resend it.")
		default:
			b.WriteString("held by the recipient's wake policy (" + string(d.Hold) + "); it waits until the person releases it. Don't resend it.")
		}
	case inbox.Queued:
		if d.To != "" && !n.live(d.To) {
			b.WriteString(s.notRunning(d, n))
		} else {
			b.WriteString("queued; they are woken with it, and their reply comes back to you as a message.")
		}
	default:
		b.WriteString(string(d.State) + ".")
	}
	// A PR message its owner couldn't take says who it was for.
	if owner := oneLine(d.For, maxFieldRunes); pr && owner != "" {
		b.WriteString(" It went to the repo instead: its owner " + owner + " isn't running now.")
	}
	return b.String()
}

// notRunning is the outcome of a message queued for an agent that isn't
// running. If its session has ended, the inbox sweep hands agent work on
// after a few minutes: to the card it is about, or to the person when an
// agent sent it. Repo and PR work moves to another desk instead, and the
// person's own message about no card waits.
func (s *set) notRunning(d inbox.Delivery, n names) string {
	const waits = "queued; they aren't running now, so it waits until they come back."
	m := d.Message
	if m.To.Kind == inbox.ToRepo || m.To.Kind == inbox.ToPR || d.RepoKey != "" || m.Notice || n.owner(d.To) {
		return waits
	}
	const lead = "queued; they aren't running now. If they don't come back within a few minutes it goes "
	if co := s.b.Coord(); co != nil {
		if tid, ok := cardFor(co, m.Ref); ok {
			return lead + "on task:" + oneLine(tid, 80) + "'s thread instead."
		}
	}
	if n.owner(m.From.Key) {
		return waits
	}
	return lead + "to the person, marked recipient gone."
}

type inboxArgs struct {
	All   bool `json:"all"`
	Limit int  `json:"limit"`
}

func (s *set) inbox(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a inboxArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	limit := a.Limit
	if limit <= 0 {
		limit = defaultInboxLimit
	}
	limit = min(limit, maxInboxLimit)
	ib := s.b.Inbox()
	// For is the recipient's own view: held deliveries never appear.
	ds := ib.For(c.Sender.Key, a.All, limit)
	what := "open messages"
	if a.All {
		what = "recent messages"
	}
	if len(ds) == 0 {
		return fmt.Sprintf("No %s for you.", what)
	}
	channel := c.Path == inbox.PathChannel
	n := s.roster()
	now := s.now()

	// Fit whole rows into MaxResult before anything is marked read, so a row
	// the result can't carry is never cut mid-way and never counted as seen.
	// Rows are sized with the state they will show after MarkRead
	// ("delivered" is the longest a queued row can become).
	head := func(shown int) string {
		if shown == len(ds) {
			return fmt.Sprintf("%d %s for you, newest first. Answer with reply (delivery_id), or use defer or skip. %s\n", len(ds), what, untrusted)
		}
		return fmt.Sprintf("%d %s for you, newest first; showing the first %d. Answer with reply (delivery_id), or use defer or skip. %s\n", len(ds), what, shown, untrusted)
	}
	foot := func(more int) string {
		if more == 0 {
			return ""
		}
		return fmt.Sprintf("\n[%d more not shown; they stay unread. Answer, defer or skip the ones above, then call inbox again.]\n", more)
	}
	budget := MaxResult - len(head(0)) - len(foot(len(ds)))
	type row struct {
		d       inbox.Delivery
		textCap int
	}
	var rows []row
	used := 0
	for _, d := range ds {
		if channel && d.State == inbox.Queued {
			d.State = inbox.Delivered
		}
		r := row{d, maxListText}
		size := len(inboxRow(n, now, r.d, r.textCap))
		// One row with heavily escaped text can outgrow the whole budget;
		// shorten the first row's text rather than show nothing.
		for len(rows) == 0 && size > budget && r.textCap > 64 {
			r.textCap /= 2
			size = len(inboxRow(n, now, r.d, r.textCap))
		}
		if used+size > budget {
			break
		}
		used += size
		rows = append(rows, r)
	}

	render := func() string {
		var b strings.Builder
		b.WriteString(head(len(rows)))
		for _, r := range rows {
			b.WriteString(inboxRow(n, now, r.d, r.textCap))
		}
		b.WriteString(foot(len(ds) - len(rows)))
		return b.String()
	}
	// The budget is an estimate (header digits vary); check the real result
	// and drop rows until it fits, so Call's clip never cuts a marked row.
	out := render()
	for len(out) > MaxResult && len(rows) > 0 {
		rows = rows[:len(rows)-1]
		out = render()
	}

	// A Grok push isn't delivered until the recipient shows it has it
	// (design §4); reading the inbox does, but only for rows shown in full.
	// A failed MarkRead shows "queued", which is shorter, so out still fits.
	if channel {
		changed := false
		for i := range rows {
			if ds[i].State == inbox.Queued && ib.MarkRead(ctx, c.Sender.Key, ds[i].ID) != nil {
				rows[i].d.State = inbox.Queued
				changed = true
			}
		}
		if changed {
			out = render()
		}
	}
	return out
}

// inboxRow renders one delivery for the inbox list, text clipped to textCap
// runes.
func inboxRow(n names, now time.Time, d inbox.Delivery, textCap int) string {
	var b strings.Builder
	m := d.Message
	fmt.Fprintf(&b, "\n- delivery %s [%s] from %s, %s, trust %s\n", d.ID, d.State, n.sender(m.From), ago(now, m.SentAt), oneLine(string(m.Trust), 20))
	switch m.To.Kind {
	case inbox.ToRepo:
		fmt.Fprintf(&b, "  to repo: %s\n", oneLine(m.To.Value, 160))
	case inbox.ToPR:
		fmt.Fprintf(&b, "  about PR: %s\n", oneLine(m.To.Value, 160))
		if owner := oneLine(d.For, maxFieldRunes); owner != "" {
			fmt.Fprintf(&b, "  for: %s, the PR's owner, who isn't running now\n", owner)
		}
	}
	if m.ReplyTo != "" {
		fmt.Fprintf(&b, "  in reply to message: %s\n", oneLine(m.ReplyTo, 80))
	}
	if d.State == inbox.Deferred && !d.Until.IsZero() {
		fmt.Fprintf(&b, "  deferred until: %s\n", in(now, d.Until))
	}
	if note := oneLine(d.Note, maxFieldRunes); note != "" {
		fmt.Fprintf(&b, "  note: %s\n", note)
	}
	if r := oneLine(m.Reason, maxFieldRunes); r != "" {
		fmt.Fprintf(&b, "  reason (untrusted): %s\n", r)
	}
	fmt.Fprintf(&b, "  text (untrusted): %s\n", quoted(m.Text, textCap))
	for _, f := range m.Attachments {
		fmt.Fprintf(&b, "  attachment (untrusted name): %s\n", oneLine(f.Describe(), 300))
	}
	return b.String()
}

type deliveryArgs struct {
	DeliveryID string `json:"delivery_id"`
	Minutes    *int   `json:"minutes"`
	Note       string `json:"note"`
}

func (s *set) ack(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a struct {
		DeliveryID string `json:"delivery_id"`
	}
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	if c.Path == inbox.PathMod {
		return errText("the Irori mod acknowledges deliveries for you; reply, defer or skip instead.")
	}
	id := strings.TrimSpace(a.DeliveryID)
	if id == "" {
		return errText("give delivery_id.")
	}
	if err := s.b.Inbox().Ack(ctx, c.Sender.Key, id); err != nil {
		return mapErr(err)
	}
	return "Acknowledged delivery " + oneLine(id, 80) + ". Reply when you have an answer."
}

func (s *set) deferTool(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a deliveryArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := strings.TrimSpace(a.DeliveryID)
	if id == "" {
		return errText("give delivery_id.")
	}
	if a.Minutes == nil {
		return errText("give minutes (1 to 720).")
	}
	if *a.Minutes < 1 || *a.Minutes > 720 {
		return errText(fmt.Sprintf("minutes must be 1 to 720 (12 hours), not %d.", *a.Minutes))
	}
	if err := s.b.Inbox().Defer(ctx, c.Sender.Key, id, *a.Minutes, strings.TrimSpace(a.Note)); err != nil {
		return mapErr(err)
	}
	return fmt.Sprintf("Deferred delivery %s for %d minutes; it comes back to you then.", oneLine(id, 80), *a.Minutes)
}

func (s *set) skip(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a struct {
		DeliveryID string `json:"delivery_id"`
		Note       string `json:"note"`
	}
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id := strings.TrimSpace(a.DeliveryID)
	if id == "" {
		return errText("give delivery_id.")
	}
	if err := s.b.Inbox().Skip(ctx, c.Sender.Key, id, strings.TrimSpace(a.Note)); err != nil {
		return mapErr(err)
	}
	return "Skipped delivery " + oneLine(id, 80) + "; it is closed without a reply."
}

func trimPtr(p *string) string {
	if p == nil {
		return ""
	}
	return strings.TrimSpace(*p)
}

package tools

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/StephenSHorton/irori/internal/inbox"
)

const (
	defaultInboxLimit = 20
	maxInboxLimit     = 50
)

type sendArgs struct {
	To     *string `json:"to"`
	Repo   *string `json:"repo"`
	Text   string  `json:"text"`
	Reason string  `json:"reason"`
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
	case to != "":
		addr = inbox.Address{Kind: inbox.ToAgent, Value: to}
	case repo != "":
		addr = inbox.Address{Kind: inbox.ToRepo, Value: repo}
	default:
		return errText("say who it's for: to = an agent's name or key, or repo = a repoKey (see the agents tool).")
	}
	if strings.TrimSpace(a.Text) == "" {
		return errText("give the message text.")
	}
	m, d, err := s.b.Inbox().Send(ctx, c.Sender, addr, a.Text, strings.TrimSpace(a.Reason))
	if err != nil {
		return mapErr(err)
	}
	return "Sent message " + m.ID + " " + s.outcome(d)
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
	return "Replied to delivery " + oneLine(id, 80) + " (now answered) with message " + m.ID + " " + s.outcome(d)
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
			b.WriteString("queued; they aren't running now, so it waits until they come back.")
		} else {
			b.WriteString("queued; they are woken with it, and their reply comes back to you as a message.")
		}
	default:
		b.WriteString(string(d.State) + ".")
	}
	return b.String()
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
	// A Grok push isn't delivered until the recipient shows it has it
	// (design §4); reading the inbox does.
	if c.Path == inbox.PathChannel {
		for i, d := range ds {
			if d.State == inbox.Queued && ib.MarkRead(ctx, c.Sender.Key, d.ID) == nil {
				ds[i].State = inbox.Delivered
			}
		}
	}
	n := s.roster()
	now := s.now()
	var b strings.Builder
	fmt.Fprintf(&b, "%d %s for you, newest first. Answer with reply (delivery_id), or use defer or skip. %s\n", len(ds), what, untrusted)
	for _, d := range ds {
		m := d.Message
		fmt.Fprintf(&b, "\n- delivery %s [%s] from %s, %s, trust %s\n", d.ID, d.State, n.sender(m.From), ago(now, m.SentAt), oneLine(string(m.Trust), 20))
		if m.To.Kind == inbox.ToRepo {
			fmt.Fprintf(&b, "  to repo: %s\n", oneLine(m.To.Value, 160))
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
		fmt.Fprintf(&b, "  text (untrusted): %s\n", quoted(m.Text, maxListText))
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

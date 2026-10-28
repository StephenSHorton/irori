package core

import (
	"context"
	"errors"
	"strings"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/tools"
)

// RouteNotices delivers coord Notices through the one notice router the
// tools and the sweep use (tools.RouteNotice: next_turn unless the notice
// asked to wake, with its Ref, marked as a notice). from is who caused it:
// the person (UserSender) from the GUI. Every recipient is tried; failures
// come back joined, since the coord row is already written.
func (c *Core) RouteNotices(ctx context.Context, from inbox.Sender, ns ...coord.Notice) error {
	var errs []error
	for _, n := range ns {
		errs = append(errs, tools.NoticeErr(c.RouteNotice(ctx, from, n)))
	}
	return errors.Join(errs...)
}

// recordGone is inbox.Options.OnRecipientGone: a message whose recipient
// session ended goes on the card its Ref names (task:<id>, or comment:<id>
// on a card), marked gone and written by Irori itself. A ref that names no
// card here is inbox.ErrNoCard, so the message goes to the person instead.
func (c *Core) recordGone(ctx context.Context, m inbox.Message, to string) (string, error) {
	tid, ok := strings.CutPrefix(m.Ref, "task:")
	if cid, isComment := strings.CutPrefix(m.Ref, "comment:"); isComment {
		cm, found := c.coord.GetComment(strings.TrimSpace(cid))
		tid, ok = cm.TaskID, found
	}
	if tid = strings.TrimSpace(tid); !ok || tid == "" {
		return "", inbox.ErrNoCard
	}
	cm, err := c.coord.RecordMessage(ctx, coord.MessageEntry{
		TaskID: tid, MsgID: m.ID, From: m.From.Key, To: to, Text: m.Text,
		Files: len(m.Attachments), Gone: true, Writer: "irori",
	})
	if errors.Is(err, coord.ErrNotFound) {
		return "", inbox.ErrNoCard
	}
	if err != nil {
		return "", err
	}
	return cm.TaskID, nil
}

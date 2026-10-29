package tools

import (
	"context"
	"encoding/json"
	"errors"
	"strings"

	"github.com/StephenSHorton/irori/internal/coord"
)

// Asks (docs/design/board.md "Asks"): the person talking to the room. One
// agent takes an ask with ask_claim, talks it through on the card, and ends
// it with ask_close. (The notify tool's approvals are another kind of ask:
// notify.go.)

// AskInstructions is how an agent treats an ask, carried by the two tools
// and by BoardInstructions.
const AskInstructions = "An ask is the person talking to the room: work to start now, not backlog. If you are free and can take it, " +
	"ask_claim it. One agent takes it, so if someone else has, leave it unless you have something to add (board_comment). " +
	"Once you hold an ask: if it isn't clear what's wanted, talk it through with the person on the card first (board_comment). " +
	"Then ask_close it with what it became: a card (it becomes the card, and you hold it), a change to an existing card, or nothing, " +
	"with one line on why."

type askClaimArgs struct {
	TaskID string `json:"task_id"`
}

func (s *set) askClaim(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a askClaimArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
	}
	co := s.b.Coord()
	r, err := co.AskClaim(ctx, c.Sender.Key, id, coord.AskWait)
	n := s.roster()
	if err != nil {
		if errors.Is(err, coord.ErrHeld) {
			if t, ok := co.GetTask(id); ok && t.Ask != nil && t.Ask.Taker != "" {
				return n.who(t.Ask.Taker) + " took " + t.ShortRef() + ". To add something, board_comment it; otherwise leave it."
			}
		}
		return mapErr(err)
	}
	ref := r.Task.ShortRef()
	switch {
	case r.Mine:
		return "You have " + ref + ". Talk it through with the person on the card first (board_comment) if it isn't clear what " +
			"they want, then ask_close it with what it became: a card, a change to another card, or nothing.\n" +
			s.cardLine(r.Task, nil, n, s.now())
	case r.Taker != "":
		return n.who(r.Taker) + " took " + ref + " first; nothing to do unless you want to chime in (board_comment)."
	}
	return "Requested. The device that hands out " + ref + " (" + oneLine(r.Owner, 80) + ") hasn't answered (offline?). " +
		"Don't start: you'll hear at your next turn either way."
}

type askCloseArgs struct {
	TaskID    string    `json:"task_id"`
	Outcome   string    `json:"outcome"`
	Note      string    `json:"note"`
	Title     string    `json:"title"`
	Objective string    `json:"objective"`
	DoneWhen  string    `json:"done_when"`
	Steps     []stepArg `json:"steps"`
	Task      string    `json:"task"`
}

func (s *set) askClose(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a askCloseArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	id, msg := s.taskID(a.TaskID)
	if msg != "" {
		return msg
	}
	kind := strings.ToLower(strings.TrimSpace(a.Outcome))
	if strings.TrimSpace(a.Note) == "" {
		return errText("give note: one line on what it became and why.")
	}
	o := coord.AskOutcome{Kind: kind, Note: a.Note, Title: a.Title, Objective: a.Objective, DoneWhen: a.DoneWhen, Steps: stepsOf(a.Steps)}
	switch kind {
	case coord.OutcomeCard:
		if strings.TrimSpace(a.Title) == "" {
			return errText("a card outcome needs title: the card's short title.")
		}
	case coord.OutcomeEdit:
		if strings.TrimSpace(a.Task) == "" {
			return errText("an edit outcome needs task: the card it changed (#12).")
		}
		target, msg := s.taskID(a.Task)
		if msg != "" {
			return msg
		}
		o.Task = target
	case coord.OutcomeNone:
	default:
		return errText("give outcome: card (it becomes a card), edit (it changes another card) or none.")
	}
	t, err := s.b.Coord().AskClose(ctx, c.Sender.Key, id, o)
	if err != nil {
		return mapErr(err)
	}
	n := s.roster()
	switch kind {
	case coord.OutcomeCard:
		return t.ShortRef() + " is a card now, and you hold it. Give it steps (board_steps) if it has none, and keep them ticked.\n" +
			s.cardLine(t, nil, n, s.now())
	case coord.OutcomeEdit:
		return "Changed " + s.shortOf(o.Task) + "; the ask " + t.ShortRef() + " is done."
	}
	return "Closed " + t.ShortRef() + ": nothing came of it."
}

// askGroups splits the Board's asks off for the board tool: open asks the
// viewer could take (no taker, in its repo or any), then the ones it took.
// They come out of the other groups.
func askGroups(v *coord.BoardView, viewer, repo string) (open, mine []coord.Card) {
	for _, g := range []*[]coord.Card{&v.NeedsYou, &v.Blocked, &v.Working, &v.Open} {
		keep := (*g)[:0:0]
		for _, cd := range *g {
			a := cd.Ask
			switch {
			case !a.Open():
				keep = append(keep, cd)
			case a.Taker == viewer:
				mine = append(mine, cd)
			case a.Taker == "" && (a.RepoKey == "" || repo == "" || a.RepoKey == repo):
				open = append(open, cd)
			default:
				keep = append(keep, cd)
			}
		}
		*g = keep
	}
	return open, mine
}

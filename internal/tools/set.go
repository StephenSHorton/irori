package tools

import (
	"bytes"
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
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/search"
)

// callerReads are the read-only tools that read as the caller.
var callerReads = []string{"inbox", "timeline", "overview", "search", "show", "board"}

// ErrNotObject means the args weren't a JSON object (Call's non-nil error).
var ErrNotObject = errors.New("tools: args must be a JSON object")

type handler func(ctx context.Context, c Caller, args json.RawMessage) string

type set struct {
	b        Backend
	now      func() time.Time
	specs    []Spec
	handlers map[string]handler
	// needsCaller: tools that act as, or read for, the caller.
	needsCaller map[string]bool
	// notifyState is the notify tool's rate window (notify.go).
	notifyState notifyState
}

func newSet(b Backend) *Set {
	s := &set{b: b, now: time.Now}
	s.handlers = map[string]handler{
		"agents":          s.agents,
		"events":          s.events,
		"send":            s.send,
		"reply":           s.reply,
		"inbox":           s.inbox,
		"ack":             s.ack,
		"defer":           s.deferTool,
		"skip":            s.skip,
		"overview":        s.overview,
		"search":          s.search,
		"show":            s.show,
		"board":           s.board,
		"board_create":    s.boardCreate,
		"board_comment":   s.boardComment,
		"board_block":     s.boardBlock,
		"board_unblock":   s.boardUnblock,
		"board_join":      s.boardJoin,
		"board_leave":     s.boardLeave,
		"board_assign":    s.boardAssign,
		"board_status":    s.boardStatus,
		"board_link":      s.boardLink,
		"board_steps":     s.boardSteps,
		"board_step":      s.boardStep,
		"board_recommend": s.boardRecommend,
		"board_watch":     s.boardWatch,
		"board_unwatch":   s.boardUnwatch,
		"commits":         s.commits,
		"prs":             s.prs,
		"lease":           s.lease,
		"leases":          s.leases,
		"release":         s.release,
		"timeline":        s.timeline,
		"why":             s.why,
		"join":            s.join,
		"notify":          s.notify,
		"attachment_get":  s.attachmentGet,
	}
	// The notify tool's asks are rows when the backend keeps them (v0.4
	// §6.7): the answer comes back from whichever device the person used.
	if book := s.asks(); book != nil {
		book.OnResolved(s.askResolved)
	}
	s.needsCaller = map[string]bool{}
	for _, sp := range specList {
		if _, ok := s.handlers[sp.Name]; !ok {
			panic("tools: no handler for " + sp.Name)
		}
		// Reads for the caller: inbox and timeline show its messages; board
		// groups "needs you" for it; overview, search and show pass it as
		// the Viewer, so they must never run as nobody (Viewer "" is the
		// person, who sees every message).
		if !sp.ReadOnly || slices.Contains(callerReads, sp.Name) {
			s.needsCaller[sp.Name] = true
		}
	}
	delete(s.needsCaller, "join")
	s.specs = specList
	return &Set{impl: s}
}

// specs lists the tools for client; "" lists every tool.
func (s *Set) specs(client string) []Spec {
	out := []Spec{}
	for _, sp := range s.impl.specs {
		if client == "" || slices.Contains(sp.Clients, client) {
			sp.Clients = slices.Clone(sp.Clients)
			sp.InputSchema = slices.Clone(sp.InputSchema)
			out = append(out, sp)
		}
	}
	return out
}

func (s *Set) call(ctx context.Context, c Caller, name string, args json.RawMessage) (string, error) {
	x := s.impl
	h, ok := x.handlers[name]
	if !ok {
		return "", fmt.Errorf("%w: %q", ErrUnknownTool, name)
	}
	args = bytes.TrimSpace(args)
	if len(args) == 0 || bytes.Equal(args, []byte("null")) {
		args = json.RawMessage("{}")
	}
	if args[0] != '{' || !json.Valid(args) {
		return "", ErrNotObject
	}
	if x.needsCaller[name] && strings.TrimSpace(c.Sender.Key) == "" {
		return errText("this connection isn't bound to an agent yet. Grok: call join first. " +
			"Claude Code: the Irori mod attaches your session; reload it if this persists."), nil
	}
	return clip(h(ctx, c, args), MaxResult), nil
}

// decode reads args into v strictly: unknown fields and wrong types are
// errors the model can fix.
func decode(args json.RawMessage, v any) error {
	dec := json.NewDecoder(bytes.NewReader(args))
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		var te *json.UnmarshalTypeError
		switch {
		case errors.As(err, &te):
			return fmt.Errorf("argument %s should be %s", te.Field, kindName(te.Type.Kind().String()))
		case strings.HasPrefix(err.Error(), "json: unknown field "):
			return fmt.Errorf("unknown argument %s", strings.TrimPrefix(err.Error(), "json: unknown field "))
		}
		return err
	}
	return nil
}

func kindName(k string) string {
	switch k {
	case "string":
		return "a string"
	case "bool":
		return "true or false"
	case "int", "int64":
		return "a whole number"
	case "slice":
		return "an array of strings"
	}
	return k
}

func errText(msg string) string { return "Error: " + oneLine(msg, 600) }

func badArgs(err error) string { return errText(err.Error()) }

// mapErr turns an inbox or coord error into advice. The error text can carry
// caller-supplied values and agent names, so it is folded onto one line.
func mapErr(err error) string {
	switch {
	case errors.Is(err, inbox.ErrRateLimited):
		return errText("rate limited: at most 8 messages per pair of agents per 10 minutes. Wait, or batch what you have into one message. (" + err.Error() + ")")
	case errors.Is(err, inbox.ErrUnknownAgent):
		return errText(err.Error() + ". Use the agents tool to see names and keys.")
	case errors.Is(err, inbox.ErrAmbiguous):
		return errText(err.Error() + ". Use the agent's key from the agents tool.")
	case errors.Is(err, inbox.ErrSelf):
		return errText("that's you; address another agent (see the agents tool).")
	case errors.Is(err, inbox.ErrEmpty):
		return errText("give the message text.")
	case errors.Is(err, inbox.ErrNotYours), errors.Is(err, inbox.ErrNotFound):
		return errText(err.Error() + ". Use a delivery id addressed to you: the delivery=\"…\" value of an <irori-message> tag, or one the inbox tool lists.")
	case errors.Is(err, inbox.ErrBadState):
		return errText(err.Error() + ". The inbox tool (all = true) shows each delivery's state.")
	case errors.Is(err, coord.ErrNotFound):
		return errText(err.Error() + ". The board and leases tools show what exists.")
	case errors.Is(err, coord.ErrNotAllowed):
		return errText(err.Error() + ". show task:<id> lists its holders and creator; comment on it or message them instead.")
	case errors.Is(err, coord.ErrHeld):
		return errText(err.Error() + ". Comment on the card or message its holder; or wait for it to come free.")
	case errors.Is(err, coord.ErrInvalid):
		return errText(err.Error() + ".")
	case errors.Is(err, search.ErrBadRef):
		return errText(err.Error() + ". Copy a ref exactly as a list line printed it.")
	case errors.Is(err, search.ErrNotFound):
		return errText(err.Error() + " that you may read. search finds refs; messages show only to their sender and recipients.")
	case errors.Is(err, capture.ErrNoRepo):
		return errText(err.Error() + ". Give an absolute path, or repo = a repoKey from the agents tool.")
	case errors.Is(err, coord.ErrBadResource):
		return errText(err.Error() + ". For files use path:<repoKey>:<glob>, with the repoKey from the agents tool.")
	}
	return errText(err.Error())
}

// names maps agent keys to display names for results.
// owner is the person's rule (coord's isOwner: "user:<device>" for a device
// certified to them), so a made-up "user:" key is never shown as the person.
type names struct {
	cards map[string]presence.AgentCard
	owner func(string) bool
}

func (s *set) roster() names {
	out := names{cards: map[string]presence.AgentCard{}, owner: s.isOwner}
	for _, c := range s.b.Agents() {
		out.cards[c.Key] = c
	}
	return out
}

// isOwner reports whether key is the person's (v0.4 §2).
func (s *set) isOwner(key string) bool {
	if co := s.b.Coord(); co != nil {
		return co.IsPerson(key)
	}
	return false
}

// who renders an agent key as `name (key)`, folded.
func (n names) who(key string) string {
	if key == "" {
		return "nobody"
	}
	k := oneLine(key, 120)
	if n.owner(key) {
		return "the person (" + k + ")"
	}
	if c, ok := n.cards[key]; ok {
		if name := oneLine(c.Name, 80); name != "" {
			if c.Status == "gone" {
				return name + " (" + k + ", gone)"
			}
			return name + " (" + k + ")"
		}
	}
	return k
}

// sender renders a message's sender: the card name when the agent is on
// the roster, else the name the Core recorded for it.
func (n names) sender(f inbox.Sender) string {
	if _, ok := n.cards[f.Key]; ok || n.owner(f.Key) {
		return n.who(f.Key)
	}
	if name := oneLine(f.Name, 80); name != "" {
		return name + " (" + oneLine(f.Key, 120) + ")"
	}
	return n.who(f.Key)
}

// short is a key's name alone when the roster knows it (the person is
// "the person"), else the key: for lines that repeat who.
func (n names) short(key string) string {
	if n.owner(key) {
		return "the person"
	}
	if c, ok := n.cards[key]; ok {
		if name := oneLine(c.Name, 80); name != "" {
			return name
		}
	}
	return oneLine(key, 120)
}

func (n names) live(key string) bool {
	c, ok := n.cards[key]
	return ok && c.Status != "gone"
}

func (s *set) join(context.Context, Caller, json.RawMessage) string {
	return errText("join is handled by irori mcp")
}

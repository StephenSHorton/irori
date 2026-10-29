package main

import (
	"cmp"
	"context"
	"fmt"
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/roster"
)

// The cards window (docs/design/v0.4-devices.md §9, v0.3.3): BoardService's
// reads and the person's answer to a blocker. Every list is non-nil, so the
// webview never gets null.

// ScopeMine is the only scope until Shared Boards land: the person's own
// devices.
const (
	ScopeMine      = "mine"
	ScopeMineLabel = "My devices"
	// recentLines is how many activity lines Card returns.
	recentLines = 8
	// MaxThreadView is how many thread entries Card returns, the newest;
	// ThreadEarlier counts the rest.
	MaxThreadView = 300
	// NoteItem is the blocker id of an input_required card's question: its
	// note, shown and answered like a blocker item.
	NoteItem = "note"
	// noteQuestion stands in for an input_required card with no note.
	noteQuestion = "The agent on it is waiting on your input."
)

// AgentRef is someone on a card, as the window shows them.
type AgentRef struct {
	Key  string `json:"key"`
	Name string `json:"name"` // the roster's name, "You" for the person, else the key
	// Status is the roster's status ("busy", "idle", ...), "gone" when the
	// roster has no live card, or "person".
	Status string `json:"status"`
	Live   bool   `json:"live"`  // in the roster and not gone (the live dot)
	Where  string `json:"where"` // the machine: "This PC", or the other device's name (v0.4)
	// Assigned: a holder that was assigned the card and hasn't taken it up.
	Assigned bool `json:"assigned,omitempty"`
	// Waiting: Assigned, and the assignment never reached this agent: its
	// notice is still queued or held in the inbox.
	Waiting bool `json:"waiting,omitempty"`
	// Device is the device chip on a face from another device: that
	// device's name, set only once the person has more than one device
	// (v0.4 §9).
	Device string `json:"device,omitempty"`
	// DeviceID is the device the agent runs on ("" for the person and
	// Irori); Color is that device's colour (devcolor.Palette), set only
	// once the person has more than one device (card #22).
	DeviceID string `json:"deviceId,omitempty"`
	Color    string `json:"color,omitempty"`
	// Offline: the host device is offline, so the face shows greyed.
	Offline bool `json:"offline,omitempty"`
	// Conflict is the resource this agent leased while partitioned from
	// another device's holder of it (v0.4 §6.4), for the conflict chip.
	Conflict string `json:"conflict,omitempty"`
}

// CardProgress is done steps over all steps; Total 0 means no bar.
type CardProgress struct {
	Done  int `json:"done"`
	Total int `json:"total"`
}

// Holding says what a card's holder is doing: the line a card without
// steps shows in place of a progress bar, so a card nobody is working
// doesn't look like work in progress.
type Holding string

const (
	HoldNobody   Holding = "nobody"   // no holder
	HoldPerson   Holding = "person"   // the person holds it
	HoldWaiting  Holding = "waiting"  // assigned, and the notice never reached the agent (queued or held)
	HoldAssigned Holding = "assigned" // assigned and told, not taken up yet
	HoldActive   Holding = "active"   // the agent working it is live and busy
	HoldIdle     Holding = "idle"     // the agent working it is live and idle
	HoldOffline  Holding = "offline"  // the agent working it is on a device that is offline
	HoldGone     Holding = "gone"     // the agent working it is gone
)

// CardHolder is the holder line: Who is the holder it describes (the
// first that took the card up, else the first assigned; zero for nobody),
// and Others counts the rest.
type CardHolder struct {
	Who    AgentRef `json:"who"`
	State  Holding  `json:"state"`
	Others int      `json:"others"`
}

// CardSummary is one card in the grid.
type CardSummary struct {
	ID string `json:"id"`
	// Short is the card's short ref without its "#" ("12", or "12b" for
	// the later of two cards numbered 12 on devices that were apart);
	// ShortWas is the number such a card showed before that settled.
	Short      string          `json:"short"`
	ShortWas   string          `json:"shortWas"`
	Title      string          `json:"title"`
	State      coord.TaskState `json:"state"`
	RepoKey    string          `json:"repoKey"`
	Scope      string          `json:"scope"`      // ScopeMine
	ScopeLabel string          `json:"scopeLabel"` // "My devices"
	Progress   CardProgress    `json:"progress"`
	// OpenBlockers counts the items waiting on a decision (open or
	// replied); BlockerQuestion is the first one's question ("" when none).
	// Waiting counts the items the person answered that wait on an agent
	// to resolve or reply.
	OpenBlockers    int        `json:"openBlockers"`
	BlockerQuestion string     `json:"blockerQuestion"`
	Waiting         int        `json:"waiting"`
	Working         []AgentRef `json:"working"`  // holders, in join order
	Watching        []AgentRef `json:"watching"` // watchers that don't hold it
	Holder          CardHolder `json:"holder"`
	UpdatedAt       time.Time  `json:"updatedAt"`
	// Returned names the agent that went away when Irori took it off and
	// nobody is on the card now (card #16); nil otherwise.
	Returned *AgentRef `json:"returned,omitempty"`
	// Stale: a working card nothing has happened on for StaleAfter and
	// whose holder isn't busy on it right now (card #16).
	Stale bool `json:"stale,omitempty"`
	// Devices are the devices its agent holders run on, in holder order
	// (the grid's device filter); Accent is their colour when they are
	// all on one device, "" otherwise or while the person has one device
	// (card #22).
	Devices []string `json:"devices"`
	Accent  string   `json:"accent,omitempty"`
	// Talking are the agents that messaged or commented to each other on
	// the card in the last TalkWindow, when there are at least two (the
	// tile's talk bubble, card #22); empty otherwise.
	Talking []AgentRef `json:"talking"`
	// Ask is set on the person's ask to the room (board.md "Asks"); nil on
	// any other card.
	Ask *CardAsk `json:"ask,omitempty"`
	// Subtasks (board.md "Subtasks"). Parent is the card this one is part
	// of; Subtasks counts a parent's, and then State and Progress come from
	// them (done/total subtasks). SubtaskBlocked is "#14: <question>" when
	// a subtask is what blocks the parent.
	Parent         *CardRef `json:"parent,omitempty"`
	Subtasks       int      `json:"subtasks,omitempty"`
	SubtaskBlocked string   `json:"subtaskBlocked,omitempty"`
	// Pinned: the person pinned it; the grid shows it in the Pinned group,
	// ordered by PinRank (board.md "Pinned cards").
	Pinned  bool    `json:"pinned,omitempty"`
	PinRank float64 `json:"pinRank,omitempty"`
}

// CardRef names another card: a subtask's parent.
type CardRef struct {
	ID    string `json:"id"`
	Short string `json:"short"`
	Title string `json:"title"`
}

// CardSubtask is one row of a parent's subtask list on its page.
type CardSubtask struct {
	ID       string          `json:"id"`
	Short    string          `json:"short"`
	Title    string          `json:"title"`
	State    coord.TaskState `json:"state"`
	Holder   CardHolder      `json:"holder"`
	Progress CardProgress    `json:"progress"`
	Blocked  bool            `json:"blocked"`
}

// Ask states, as the tile's line shows them.
const (
	AskOpen      = "open"      // agents heard it, nobody took it yet
	AskWaiting   = "waiting"   // a claim waits on the device that hands it out
	AskUnclaimed = "unclaimed" // nobody took it in time (or nobody was free)
	AskTaken     = "taken"     // an agent took it
	AskClosed    = "closed"    // it ended: Outcome says how
)

// CardAsk is an ask on its tile and page: where it stands, who took it,
// how many heard it, the device that hands it out, and once closed what it
// became (OutcomeRef: the card an edit changed, "#12").
type CardAsk struct {
	State      string         `json:"state"`
	Taker      *AgentRef      `json:"taker,omitempty"`
	Told       int            `json:"told"`
	Owner      string         `json:"owner"` // the device's name ("This PC" for this one)
	Wake       bool           `json:"wake"`
	Outcome    *coord.Outcome `json:"outcome,omitempty"`
	OutcomeRef string         `json:"outcomeRef"`
}

// cardAsk is t's CardAsk, or nil when t isn't an ask. ref names another
// card by id (nil: none).
func cardAsk(t coord.Task, p people, ref func(id string) string) *CardAsk {
	a := t.Ask
	if a == nil {
		return nil
	}
	out := &CardAsk{Told: a.Told, Owner: p.where(a.Owner), Wake: a.Wake}
	switch {
	case a.Outcome != nil:
		o := *a.Outcome
		out.State, out.Outcome = AskClosed, &o
		if o.Task != "" && ref != nil {
			out.OutcomeRef = ref(o.Task)
		}
	case a.Taker != "":
		r := p.ref(a.Taker)
		out.State, out.Taker = AskTaken, &r
	case len(a.Waiting) > 0:
		out.State = AskWaiting
	case a.Unclaimed:
		out.State = AskUnclaimed
	default:
		out.State = AskOpen
	}
	return out
}

// CardOption is one answer to pick. Recommended marks the one its agent
// recommends (the window shows it first-class); others are offered choices.
type CardOption struct {
	Answer      string    `json:"answer"`
	Why         string    `json:"why"`
	By          AgentRef  `json:"by"`
	At          time.Time `json:"at"`
	Recommended bool      `json:"recommended"`
	// Link is where the answer is carried out or explained, if anywhere.
	Link *coord.Link `json:"link,omitempty"`
}

// CardResolution is how a blocker was resolved.
type CardResolution struct {
	Answer string    `json:"answer"`
	Option int       `json:"option"` // index into Options, or -1 for own words or a close
	By     AgentRef  `json:"by"`
	At     time.Time `json:"at"`
	Closed bool      `json:"closed"` // closed without an answer (unblocked, state change, Irori)
	// Note is the resolver's line beside the check once the person had
	// answered (Answer stays the person's); "" before PR B or when none.
	Note string `json:"note"`
}

// Blocker states, CardBlocker.State: open, answered (the person answered;
// it waits on the raiser), replied (an agent replied to the answer; back
// with the person), resolved, or closed without an answer.
const (
	BlockerOpen     = "open"
	BlockerAnswered = "answered"
	BlockerReplied  = "replied"
	BlockerResolved = "resolved"
	BlockerClosed   = "closed"
)

// CardBlocker is one blocker item, open or resolved.
type CardBlocker struct {
	ID       string          `json:"id"`
	Question string          `json:"question"`
	Context  string          `json:"context"`
	RaisedBy AgentRef        `json:"raisedBy"`
	At       time.Time       `json:"at"`
	Options  []CardOption    `json:"options"`
	Resolved *CardResolution `json:"resolved"` // null while open
	State    string          `json:"state"`    // BlockerOpen ... BlockerClosed
	// Links and Attachments are what the raiser put at hand: pages to
	// open, paths to copy, files to look at.
	Links       []coord.Link   `json:"links"`
	Attachments []blob.FileRef `json:"attachments"`
	// SentTo is who the person's latest answer went to (the raiser), while
	// answered; Missing is what an agent's reply says is still needed,
	// while replied.
	SentTo  *AgentRef `json:"sentTo,omitempty"`
	Missing []string  `json:"missing"`
	// Thread is the item's own thread, oldest first: the person's answers
	// and the agents' replies. The card's thread keeps a line for each.
	Thread []CardThreadItem `json:"thread"`
}

// CardPR is a pull request linked from the card, with what capture's PR
// ledger knows: who opened it, reviewed it or otherwise touched it. CI and
// review state are not read from GitHub in this release.
// TODO(v0.4 People, decision 29): peer review requests to other people's
// agents ("Ask for a review") once Shared Boards land.
type CardPR struct {
	Repo       string     `json:"repo"` // "owner/name"
	Number     int        `json:"number"`
	URL        string     `json:"url"`
	Label      string     `json:"label"` // "GitHub #57"
	OpenedBy   []AgentRef `json:"openedBy"`
	ReviewedBy []AgentRef `json:"reviewedBy"`
	TouchedBy  []AgentRef `json:"touchedBy"`
}

// CardActivity is one line of Recent: Text is Who's name and what they did.
type CardActivity struct {
	At   time.Time `json:"at"`
	Who  AgentRef  `json:"who"`
	Text string    `json:"text"`
}

// Thread entry kinds: a comment on the card, a message about it (an inbox
// message mirrored onto the card, From -> To), or one line of what happened.
const (
	ThreadComment = "comment"
	ThreadMessage = "message"
	ThreadEvent   = "event"
)

// CardThreadItem is one entry of a card's thread: comments, messages about
// the card and its activity lines, merged in log order.
type CardThreadItem struct {
	ID   string    `json:"id"`
	Kind string    `json:"kind"` // ThreadComment, ThreadMessage or ThreadEvent
	At   time.Time `json:"at"`
	Who  AgentRef  `json:"who"`
	To   *AgentRef `json:"to,omitempty"` // a message's recipient
	// Device is the device that wrote the entry, for its chip: "" for this
	// one, or until the person has a second device.
	Device string `json:"device,omitempty"`
	// Text is a comment's or message's body (empty for a held message), or
	// an event's line, Who's name first.
	Text string `json:"text"`
	// BlockerID is the item a raise line raised, or the one an answer or
	// reply (or its line) is on. Missing is a reply's still-needed list.
	BlockerID string   `json:"blockerId,omitempty"`
	Missing   []string `json:"missing,omitempty"`
	// Gone: the recipient had ended, so Irori left the message on the card.
	// Held: the wake policy hasn't let it through yet, so its text isn't on
	// the card. Files counts the message's attachments, which stay on the
	// message. Message opens a message in full.
	Gone        bool           `json:"gone,omitempty"`
	Held        bool           `json:"held,omitempty"`
	Files       int            `json:"files,omitempty"`
	Attachments []blob.FileRef `json:"attachments,omitempty"` // a comment's files
	// Unheard: the person's comment had no agent around to hear it, and no
	// agent has written on the card since.
	Unheard bool `json:"unheard,omitempty"`
}

// CardMessage is a message on a card's thread in full, for the person: the
// thread shows its text cut past coord.MaxComment, none until the wake
// policy let it through, and its files only as a count.
type CardMessage struct {
	Text  string   `json:"text"`
	Files []string `json:"files"` // one line per attachment: name, size, type, hash
}

// CardDetail is one card opened beside the grid.
type CardDetail struct {
	CardSummary
	Goal     string          `json:"goal"` // the objective
	DoneWhen string          `json:"doneWhen"`
	Note     string          `json:"note"`
	Steps    []coord.Step    `json:"steps"`
	Blockers []CardBlocker   `json:"blockers"` // open first (oldest first), then resolved (newest first)
	PRs      []CardPR        `json:"prs"`
	Links    []coord.Link    `json:"links"`  // every link, PRs too
	Recent   []CardActivity  `json:"recent"` // newest first; the window shows Thread
	Comments []coord.Comment `json:"comments"`
	// Thread is comments, messages and activity, oldest first: the newest
	// MaxThreadView, with ThreadEarlier counting the ones left off.
	// HistoryTrimmed: the card has more activity than coord keeps, so its
	// oldest lines are gone.
	Thread         []CardThreadItem `json:"thread"`
	ThreadEarlier  int              `json:"threadEarlier"`
	HistoryTrimmed bool             `json:"historyTrimmed"`
	// SubtaskList is a parent's subtasks, in order, for its page.
	SubtaskList []CardSubtask `json:"subtaskList"`
}

// people resolves keys against the roster once per call.
type people struct {
	byKey map[string]presence.AgentCard
	user  string
	// owner is coord's person rule (v0.4 §2); nil is exactly user.
	owner func(string) bool
	// self is this device's id; names are the person's devices' labels.
	self  string
	names map[string]string
	// paired: the person has more than one device, so faces from another
	// device get a device chip (v0.4 §9).
	paired bool
	// colors is each device's colour (card #22).
	colors map[string]string
	// conflicts maps a holder to the resource of its contested lease.
	conflicts map[string]string
	// unsent holds "<agent>\x00<ref>" for each assignment notice (reason
	// "assigned") still queued or held for that agent: an assignment that
	// never reached it. Later notices on the card don't count.
	unsent map[string]bool
}

func unsentKey(agent, ref string) string { return agent + "\x00" + ref }

func (b *BoardService) people() people {
	p := people{byKey: map[string]presence.AgentCard{}, user: b.user.Key, self: b.self,
		names: map[string]string{}, conflicts: map[string]string{}, unsent: map[string]bool{}}
	if b.deliveries != nil {
		for _, d := range b.deliveries() {
			m := d.Message
			if m.Notice && m.Reason == string(coord.NoticeAssigned) && (d.State == inbox.Queued || d.State == inbox.Held) && strings.HasPrefix(m.Ref, "task:") {
				p.unsent[unsentKey(d.To, d.Message.Ref)] = true
			}
		}
	}
	if b.coord != nil {
		p.owner = b.coord.IsPerson
	}
	if b.leases != nil {
		for _, l := range b.leases() {
			if l.Conflict != "" {
				p.conflicts[l.Holder] = l.Resource
			}
		}
	}
	if b.agents != nil {
		for _, a := range b.agents() {
			p.byKey[a.Key] = a
		}
	}
	if b.colors != nil {
		p.colors = b.colors()
	}
	if b.devices != nil {
		live := 0
		for _, d := range b.devices() {
			p.names[d.Cert.DeviceID] = d.Cert.Label
			if !d.Revoked {
				live++
			}
		}
		p.paired = live > 1
	}
	return p
}

// where names the device a key lives on: "This PC" for this one (or when
// this device is unknown, as in a Core-less test), else its label
// ("Another device" when it has none).
func (p people) where(dev string) string {
	if dev == "" || p.self == "" || dev == p.self {
		return "This PC"
	}
	return cmp.Or(p.names[dev], "Another device")
}

// chip is the device chip for something dev wrote: its name once the
// person has a second device, "" for this device or while unpaired.
func (p people) chip(dev string) string {
	if !p.paired || dev == "" || dev == p.self {
		return ""
	}
	return p.where(dev)
}

func (p people) ref(key string) AgentRef {
	switch {
	case key == "irori":
		return AgentRef{Key: key, Name: "Irori", Status: "irori", Live: true, Where: "This PC"}
	case key == p.user:
		return AgentRef{Key: key, Name: "You", Status: "person", Live: true, Where: "This PC"}
	case p.owner != nil && p.owner(key):
		// The person at another of their devices.
		return AgentRef{Key: key, Name: "You", Status: "person", Live: true,
			Where: p.where(strings.TrimPrefix(key, identity.UserKeyPrefix))}
	}
	dev, _, ok := strings.Cut(key, "/") // "<deviceId>/<sessionId>"
	if !ok {
		dev = ""
	}
	r := AgentRef{Key: key, Name: key, Status: "gone", Conflict: p.conflicts[key]}
	if a, ok := p.byKey[key]; ok {
		if a.Name != "" {
			r.Name = a.Name
		}
		r.Status = a.Status
		r.Live = a.Status != "gone"
		r.Offline = a.DeviceState == roster.Offline
		dev = cmp.Or(a.DeviceID, dev)
	}
	r.Where = p.where(dev)
	r.Device = p.chip(dev)
	r.DeviceID = dev
	if p.paired {
		r.Color = p.colors[dev]
	}
	return r
}

func (p people) refs(keys []string) []AgentRef {
	out := []AgentRef{}
	for _, k := range keys {
		out = append(out, p.ref(k))
	}
	return out
}

func cardSummary(t coord.Task, p people) CardSummary {
	d, n := t.Progress()
	s := CardSummary{
		ID: t.ID, Short: t.Short, ShortWas: t.ShortWas, Title: t.Title, State: coord.EffectiveState(t), RepoKey: t.RepoKey,
		Scope: ScopeMine, ScopeLabel: ScopeMineLabel,
		Progress: CardProgress{Done: d, Total: n}, Working: []AgentRef{}, Watching: []AgentRef{},
		UpdatedAt: t.UpdatedAt, Subtasks: len(t.Subtasks), Pinned: t.Pinned, PinRank: t.PinRank,
	}
	if r := t.Rollup; r != nil {
		// A parent's progress is its subtasks'; its own steps came before
		// the split.
		s.Progress = CardProgress{Done: r.Done, Total: r.Total}
	}
	if open := t.OpenBlockers(); len(open) > 0 {
		// An answered item waits on an agent, not on the person; a card
		// with only those is no decision, so the Blocked fallback is not
		// for it.
		for _, bl := range open {
			if bl.Status == coord.BlockerAnswered {
				s.Waiting++
				continue
			}
			s.OpenBlockers++
			if s.OpenBlockers == 1 {
				s.BlockerQuestion = bl.Question
			}
		}
	} else if t.State == coord.Blocked {
		s.OpenBlockers, s.BlockerQuestion = 1, t.Reason
	} else if t.State == coord.InputRequired {
		s.OpenBlockers, s.BlockerQuestion = 1, cmp.Or(strings.TrimSpace(t.Note), noteQuestion)
	}
	for _, h := range t.Holders {
		r := p.ref(h)
		r.Assigned = slices.Contains(t.Assigned, h)
		r.Waiting = r.Assigned && p.unsent[unsentKey(h, "task:"+t.ID)]
		s.Working = append(s.Working, r)
	}
	for _, w := range t.Watchers {
		if !slices.Contains(t.Holders, w) {
			s.Watching = append(s.Watching, p.ref(w))
		}
	}
	s.Holder = holderLine(s.Working)
	s.Devices = []string{}
	for _, r := range s.Working {
		if r.DeviceID != "" && !slices.Contains(s.Devices, r.DeviceID) {
			s.Devices = append(s.Devices, r.DeviceID)
		}
	}
	if len(s.Devices) == 1 && p.paired {
		s.Accent = p.colors[s.Devices[0]]
	}
	s.Ask = cardAsk(t, p, nil)
	if t.Returned != nil && len(t.Holders) == 0 {
		r := p.ref(t.Returned.Was)
		s.Returned = &r
	}
	return s
}

// TalkWindow is how recent agents' messages to each other on a card must
// be for its tile to show them talking (card #22).
const TalkWindow = 5 * time.Minute

// maxTalking caps the faces in a talk bubble.
const maxTalking = 4

// talking is the agents that wrote to each other on a card (its comments,
// mirrored messages A -> B and blocker replies) within TalkWindow of now,
// newest first, when there are at least two; the person and Irori don't
// count.
func talking(cs []coord.Comment, p people, now time.Time) []AgentRef {
	var keys []string
	add := func(k string) {
		if k == "" || k == "irori" || k == p.user || (p.owner != nil && p.owner(k)) || slices.Contains(keys, k) {
			return
		}
		keys = append(keys, k)
	}
	for i := len(cs) - 1; i >= 0; i-- {
		c := cs[i]
		if now.Sub(c.At) > TalkWindow {
			break
		}
		add(c.By)
		add(c.To)
	}
	if len(keys) < 2 {
		return []AgentRef{}
	}
	if len(keys) > maxTalking {
		keys = keys[:maxTalking]
	}
	return p.refs(keys)
}

// StaleAfter: a working card with no activity this long, and nobody busy
// on it, shows as stale (card #16).
const StaleAfter = 2 * time.Hour

// stale reports whether s (a card last active at last) is stale at now.
func stale(s CardSummary, last, now time.Time) bool {
	if s.State != coord.Working || s.Holder.State == HoldActive || s.Holder.State == HoldPerson {
		return false
	}
	return !last.IsZero() && now.Sub(last) >= StaleAfter
}

// holderLine describes the holder that took the card up (the first), else
// the first assigned one.
func holderLine(working []AgentRef) CardHolder {
	if len(working) == 0 {
		return CardHolder{State: HoldNobody}
	}
	i := slices.IndexFunc(working, func(r AgentRef) bool { return !r.Assigned })
	if i < 0 {
		i = 0
	}
	h := CardHolder{Who: working[i], Others: len(working) - 1}
	switch w := h.Who; {
	case w.Status == "person":
		h.State = HoldPerson
	case w.Waiting:
		h.State = HoldWaiting
	case w.Assigned:
		h.State = HoldAssigned
	case w.Status == "gone":
		h.State = HoldGone
	case w.Offline:
		h.State = HoldOffline
	case w.Status == "idle":
		h.State = HoldIdle
	default:
		h.State = HoldActive
	}
	return h
}

// Cards lists every live card (not completed, failed or canceled) for the
// grid: cards waiting on a decision first, then the newest change first.
func (b *BoardService) Cards() []CardSummary {
	p := b.people()
	out := []CardSummary{}
	now := time.Now()
	tasks := b.coord.ListTasks(coord.TaskFilter{Open: true})
	for _, t := range tasks {
		s := cardSummary(t, p)
		last := t.UpdatedAt
		if acts := b.coord.Activity(t.ID); len(acts) > 0 && acts[0].At.After(last) {
			last = acts[0].At
		}
		// A parent's work happens on its subtasks, which show their own.
		s.Stale = s.Subtasks == 0 && stale(s, last, now)
		s.Talking = talking(b.coord.Comments(t.ID), p, now)
		b.familyOf(t, &s)
		out = append(out, s)
	}
	sort.SliceStable(out, func(i, j int) bool {
		// An open ask is the person talking to the room now: first.
		if ai, aj := openAskSummary(out[i]), openAskSummary(out[j]); ai != aj {
			return ai
		}
		if (out[i].OpenBlockers > 0) != (out[j].OpenBlockers > 0) {
			return out[i].OpenBlockers > 0
		}
		if !out[i].UpdatedAt.Equal(out[j].UpdatedAt) {
			return out[i].UpdatedAt.After(out[j].UpdatedAt)
		}
		return out[i].ID > out[j].ID
	})
	// Pinned cards come first, in the person's order, each pinned parent
	// followed by its subtasks; the rest as sorted, with each family
	// together.
	var pins []CardSummary
	byID := map[string]CardSummary{}
	for _, s := range out {
		byID[s.ID] = s
		if s.Pinned {
			pins = append(pins, s)
		}
	}
	sort.SliceStable(pins, func(i, j int) bool { return pins[i].PinRank < pins[j].PinRank })
	kids := map[string][]string{}
	for _, t := range tasks {
		kids[t.ID] = t.Subtasks
	}
	pinned, used := []CardSummary{}, map[string]bool{}
	for _, s := range pins {
		if used[s.ID] {
			continue
		}
		pinned = append(pinned, s)
		used[s.ID] = true
		for _, k := range kids[s.ID] {
			if ks, ok := byID[k]; ok && !ks.Pinned {
				pinned = append(pinned, ks)
				used[k] = true
			}
		}
	}
	rest := []CardSummary{}
	for _, s := range out {
		if !used[s.ID] {
			rest = append(rest, s)
		}
	}
	return append(pinned, families(rest, tasks)...)
}

func openAskSummary(s CardSummary) bool { return s.Ask != nil && s.Ask.State != AskClosed }

// DonePage is one page of the Done tab: Next is the cursor for the page
// after it ("" when this is the last).
type DonePage struct {
	Cards []CardSummary `json:"cards"`
	Next  string        `json:"next"`
}

// DoneCards lists the closed cards (completed, canceled or failed) for the
// Done tab, latest change first: the order the agents' board done list
// uses, which is when the card closed unless something (a comment) came
// later. limit is 0 for coord.DefaultDoneCard, at most coord.MaxDoneCard;
// before is "" for the first page, else the Next of the page before.
func (b *BoardService) DoneCards(limit int, before string) (DonePage, error) {
	if limit <= 0 {
		limit = coord.DefaultDoneCard
	}
	limit = min(limit, coord.MaxDoneCard)
	var (
		at     time.Time
		atID   string
		cursor = strings.TrimSpace(before) != ""
	)
	if cursor {
		ts, id, ok := strings.Cut(strings.TrimSpace(before), " ")
		var err error
		if at, err = time.Parse(time.RFC3339Nano, ts); !ok || err != nil || id == "" {
			return DonePage{}, fmt.Errorf("%w: cursor %q", coord.ErrInvalid, before)
		}
		atID = id
	}
	var done []coord.Task
	for _, t := range b.coord.ListTasks(coord.TaskFilter{}) {
		if coord.IsTerminal(coord.EffectiveState(t)) { // a parent ends with its subtasks
			done = append(done, t)
		}
	}
	newer := func(x, y coord.Task) bool { // x before y in the list
		if !x.UpdatedAt.Equal(y.UpdatedAt) {
			return x.UpdatedAt.After(y.UpdatedAt)
		}
		return x.ID > y.ID
	}
	sort.SliceStable(done, func(i, j int) bool { return newer(done[i], done[j]) })
	if cursor {
		mark := coord.Task{ID: atID, UpdatedAt: at}
		i, _ := slices.BinarySearchFunc(done, mark, func(t, m coord.Task) int {
			if t.ID == m.ID && t.UpdatedAt.Equal(m.UpdatedAt) {
				return 0
			}
			if newer(t, m) {
				return -1
			}
			return 1
		})
		if i < len(done) && done[i].ID == atID && done[i].UpdatedAt.Equal(at) {
			i++
		}
		done = done[i:]
	}
	out := DonePage{Cards: []CardSummary{}}
	if len(done) > limit {
		last := done[limit-1]
		out.Next = last.UpdatedAt.UTC().Format(time.RFC3339Nano) + " " + last.ID
		done = done[:limit]
	}
	p := b.people()
	ref := func(id string) string {
		if x, ok := b.coord.GetTask(id); ok {
			return x.ShortRef()
		}
		return ""
	}
	for _, t := range done {
		s := cardSummary(t, p)
		s.Ask = cardAsk(t, p, ref)
		out.Cards = append(out.Cards, s)
	}
	return out, nil
}

// familyOf fills a card's subtask fields that need other cards: its
// parent's ref, and the subtask that blocks a parent.
func (b *BoardService) familyOf(t coord.Task, s *CardSummary) {
	if pt, ok := b.coord.GetTask(t.Parent); ok && t.Parent != "" {
		s.Parent = &CardRef{ID: pt.ID, Short: pt.Short, Title: pt.Title}
	}
	if r := t.Rollup; r != nil && r.Blocked != "" {
		if k, ok := b.coord.GetTask(r.Blocked); ok {
			s.SubtaskBlocked = k.ShortRef() + ": " + r.BlockedQuestion
		}
	}
}

// families keeps a parent's subtasks right after it, in its subtask order,
// and moves the family to where its most pressing card sorted (a blocked
// subtask brings its parent up). A subtask whose parent isn't listed stays
// where it sorted.
func families(sorted []CardSummary, tasks []coord.Task) []CardSummary {
	kids := map[string][]string{}
	for _, t := range tasks {
		if len(t.Subtasks) > 0 {
			kids[t.ID] = t.Subtasks
		}
	}
	at := make(map[string]int, len(sorted))
	for i, s := range sorted {
		at[s.ID] = i
	}
	head := func(s CardSummary) string { // the card a family sorts under
		if s.Parent != nil {
			if _, ok := at[s.Parent.ID]; ok {
				return s.Parent.ID
			}
		}
		return s.ID
	}
	out := make([]CardSummary, 0, len(sorted))
	placed := map[string]bool{}
	for _, s := range sorted {
		h := head(s)
		if placed[h] {
			continue
		}
		placed[h] = true
		out = append(out, sorted[at[h]])
		for _, k := range kids[h] {
			if i, ok := at[k]; ok {
				out = append(out, sorted[i])
			}
		}
	}
	return out
}

// decideCount counts the cards waiting on a decision, the Needs a decision
// filter's number and the tray's. Rows from every device fold into the
// board, so it covers all of them (v0.4 §9).
func (b *BoardService) decideCount() int {
	n := 0
	for _, t := range b.coord.ListTasks(coord.TaskFilter{Open: true}) {
		if cardSummary(t, people{}).OpenBlockers > 0 {
			n++
		}
	}
	return n
}

// Card opens one card: goal, steps, blockers with their recommended answers
// and resolutions, who is on it, PRs, recent activity and comments. Any
// card can be opened, done ones too.
func (b *BoardService) Card(id string) (CardDetail, error) {
	t, ok := b.coord.GetTask(strings.TrimSpace(id))
	if !ok {
		return CardDetail{}, fmt.Errorf("%w: task %q", coord.ErrNotFound, id)
	}
	p := b.people()
	d := CardDetail{
		CardSummary: cardSummary(t, p), Goal: t.Objective, DoneWhen: t.DoneCriteria, Note: t.Note,
		Steps: nonNil(t.Steps), Blockers: []CardBlocker{}, PRs: []CardPR{}, Links: nonNil(t.Links),
		Recent: []CardActivity{}, Comments: nonNil(b.coord.Comments(t.ID)), SubtaskList: []CardSubtask{},
	}
	b.familyOf(t, &d.CardSummary)
	for _, id := range t.Subtasks {
		k, ok := b.coord.GetTask(id)
		if !ok {
			continue
		}
		ks := cardSummary(k, p)
		d.SubtaskList = append(d.SubtaskList, CardSubtask{ID: k.ID, Short: k.Short, Title: k.Title, State: ks.State,
			Holder: ks.Holder, Progress: ks.Progress, Blocked: ks.OpenBlockers > 0})
	}
	d.Ask = cardAsk(t, p, func(id string) string {
		if x, ok := b.coord.GetTask(id); ok {
			return x.ShortRef()
		}
		return ""
	})
	acts := b.coord.Activity(t.ID)
	var byBlocker map[string][]CardThreadItem
	d.Thread, byBlocker, d.ThreadEarlier = buildThread(d.Comments, acts, t, p)
	d.HistoryTrimmed = len(acts) == coord.MaxActivity
	var resolved []CardBlocker
	if t.State == coord.InputRequired {
		d.Blockers = append(d.Blockers, CardBlocker{ID: NoteItem, Question: d.BlockerQuestion,
			RaisedBy: p.ref(t.UpdatedBy), At: t.UpdatedAt, Options: []CardOption{}, State: BlockerOpen,
			Missing: []string{}, Thread: []CardThreadItem{}, Links: []coord.Link{}, Attachments: []blob.FileRef{}})
	}
	for _, bl := range t.Blockers {
		cb := CardBlocker{ID: bl.ID, Question: bl.Question, Context: bl.Context, RaisedBy: p.ref(bl.RaisedBy), At: bl.At,
			Options: []CardOption{}, State: cmp.Or(string(bl.Status), BlockerOpen), Missing: nonNil(bl.Missing),
			Thread: nonNil(byBlocker[bl.ID]), Links: nonNil(bl.Links), Attachments: nonNil(bl.Attachments)}
		for _, o := range bl.Options {
			cb.Options = append(cb.Options, CardOption{Answer: o.Answer, Why: o.Why, By: p.ref(o.By), At: o.At, Recommended: o.Recommended,
				Link: o.Link})
		}
		if r := bl.Resolved; r != nil {
			cb.Resolved = &CardResolution{Answer: r.Answer, Option: r.Option, By: p.ref(r.By), At: r.At, Closed: r.Closed, Note: r.Note}
			cb.State, cb.Missing = BlockerResolved, []string{}
			if r.Closed {
				cb.State = BlockerClosed
			}
			resolved = append(resolved, cb)
			continue
		}
		if cb.State == BlockerAnswered && bl.SentTo != "" {
			to := p.ref(bl.SentTo)
			cb.SentTo = &to
		}
		d.Blockers = append(d.Blockers, cb)
	}
	slices.Reverse(resolved)
	d.Blockers = append(d.Blockers, resolved...)
	for _, l := range t.Links {
		if pr, ok := b.cardPR(l, p); ok {
			d.PRs = append(d.PRs, pr)
		}
	}
	if len(acts) > recentLines {
		acts = acts[:recentLines]
	}
	for _, a := range acts {
		who := p.ref(a.By)
		d.Recent = append(d.Recent, CardActivity{At: a.At, Who: who, Text: who.Name + " " + a.What})
	}
	return d, nil
}

// Message opens a message on a card's thread in full: its whole text and
// its files, which stay on the message. Only a message that thread carries
// opens, so this reads no other message.
func (b *BoardService) Message(taskID, msgID string) (CardMessage, error) {
	taskID, msgID = strings.TrimSpace(taskID), strings.TrimSpace(msgID)
	cm, ok := b.coord.GetComment(msgID)
	if !ok || cm.MsgID == "" || cm.TaskID != taskID {
		return CardMessage{}, fmt.Errorf("%w: no message %q on task %q", coord.ErrNotFound, msgID, taskID)
	}
	var m inbox.Message
	if b.message != nil {
		m, ok = b.message(cm.MsgID)
	}
	if !ok {
		return CardMessage{}, fmt.Errorf("%w: message %q isn't on this device yet", coord.ErrNotFound, msgID)
	}
	out := CardMessage{Text: m.Text, Files: []string{}}
	for _, f := range m.Attachments {
		out.Files = append(out.Files, f.Describe())
	}
	return out, nil
}

// threadPos is an entry's place in the log, (HLC, device, seq), the order
// coord folds in. It stays server-side: an HLC overflows a JS number.
type threadPos struct {
	hlc hlc.Timestamp
	dev string
	seq int64
}

func (a threadPos) before(b threadPos) bool {
	if a.hlc != b.hlc {
		return a.hlc < b.hlc
	}
	if a.dev != b.dev {
		return a.dev < b.dev
	}
	return a.seq < b.seq
}

// buildThread merges a card's comments and its activity (newest first, as
// coord.Activity returns it) into one thread in log order, and keeps the
// newest MaxThreadView; earlier counts the rest. A comment's own
// "commented" line is left out: the comment is the entry. A blocker's
// answers and replies go in byBlocker, that item's own thread, and the
// card's thread keeps their "answered"/"replied" lines; once t no longer
// keeps the item (KeepResolved), they stay in the card's thread.
func buildThread(comments []coord.Comment, acts []coord.Activity, t coord.Task, p people) (items []CardThreadItem, byBlocker map[string][]CardThreadItem, earlier int) {
	type entry struct {
		pos  threadPos
		item CardThreadItem
	}
	kept := map[string]bool{}
	for _, bl := range t.Blockers {
		kept[bl.ID] = true
	}
	all := make([]entry, 0, len(comments)+len(acts))
	var mine []entry
	for _, cm := range comments {
		it := CardThreadItem{ID: cm.ID, Kind: ThreadComment, At: cm.At, Who: p.ref(cm.By),
			Device: p.chip(cm.Device), Text: cm.Text, Gone: cm.Gone, Held: cm.Held, Files: cm.Files,
			Attachments: cm.Attachments, BlockerID: cm.BlockerID, Missing: cm.Missing, Unheard: cm.Unheard}
		if cm.MsgID != "" {
			it.Kind = ThreadMessage
		}
		if cm.To != "" {
			to := p.ref(cm.To)
			it.To = &to
		}
		e := entry{threadPos{cm.HLC, cm.Device, cm.Seq}, it}
		if kept[cm.BlockerID] {
			mine = append(mine, e)
			continue
		}
		all = append(all, e)
	}
	byBlocker = map[string][]CardThreadItem{}
	sort.SliceStable(mine, func(i, j int) bool { return mine[i].pos.before(mine[j].pos) })
	for _, e := range mine {
		byBlocker[e.item.BlockerID] = append(byBlocker[e.item.BlockerID], e.item)
	}
	seen := map[string]int{}
	for i := len(acts) - 1; i >= 0; i-- { // oldest first, so a row's lines keep their order
		a := acts[i]
		if a.Kind == coord.KindTaskComment && a.BlockerID == "" {
			continue
		}
		// One row can fold to more than one line; the id stays unique.
		id := fmt.Sprintf("%s:%d", a.Device, a.Seq)
		if n := seen[id]; n > 0 {
			seen[id]++
			id = fmt.Sprintf("%s.%d", id, n)
		} else {
			seen[id] = 1
		}
		who := p.ref(a.By)
		all = append(all, entry{threadPos{a.HLC, a.Device, a.Seq}, CardThreadItem{ID: id, Kind: ThreadEvent,
			At: a.At, Who: who, Device: p.chip(a.Device), Text: who.Name + " " + a.What, BlockerID: a.BlockerID}})
	}
	sort.SliceStable(all, func(i, j int) bool { return all[i].pos.before(all[j].pos) })
	// An agent's comment or message after an unheard comment means an
	// agent has read the card since.
	agentLater := false
	for i := len(all) - 1; i >= 0; i-- {
		it := &all[i].item
		if it.Kind == ThreadEvent {
			continue
		}
		it.Unheard = it.Unheard && !agentLater
		agentLater = agentLater || it.Who.Status != "person" && it.Who.Status != "irori"
	}
	if n := len(all) - MaxThreadView; n > 0 {
		earlier, all = n, all[n:]
	}
	items = make([]CardThreadItem, 0, len(all))
	for _, e := range all {
		items = append(items, e.item)
	}
	return items, byBlocker, earlier
}

// cardPR reads a GitHub PR link and what the ledger knows of it.
func (b *BoardService) cardPR(l coord.Link, p people) (CardPR, bool) {
	repo, n, ok := coord.PRLink(l)
	if !ok {
		return CardPR{}, false
	}
	pr := CardPR{Repo: repo, Number: n, URL: l.Ref, Label: l.Label, OpenedBy: []AgentRef{}, ReviewedBy: []AgentRef{}, TouchedBy: []AgentRef{}}
	if b.prs == nil {
		return pr, true
	}
	key := ledger.Key(repo, n)
	seen := map[string]bool{}
	for _, e := range b.prs(repo) {
		if e.Key != key || seen[string(e.Role)+"\x00"+e.Session] {
			continue
		}
		seen[string(e.Role)+"\x00"+e.Session] = true
		r := p.ref(e.Session)
		switch e.Role {
		case ledger.Author:
			pr.OpenedBy = append(pr.OpenedBy, r)
		case ledger.Reviewer:
			pr.ReviewedBy = append(pr.ReviewedBy, r)
		default:
			pr.TouchedBy = append(pr.TouchedBy, r)
		}
	}
	return pr, true
}

// AnswerResult is Answer's reply: the card after the answer, and who heard.
type AnswerResult struct {
	Card CardDetail `json:"card"`
	Told Told       `json:"told"`
}

// Answer is the person's answer to one blocker: option is the index of
// the recommended answer picked, or -1 to answer with text. The answer
// reaches the agent that raised it (decision 18), then the holders: a
// wake when wake is true (the window's default, since that agent is
// blocked on the person), else at its next turn. The item stays open
// (answered) until the raiser, a holder or the person resolves it; when
// nobody can act on it (the raiser isn't a live agent and no holder is
// live) the answer resolves it at once, and the card leaves blocked when
// no blocker is left open.
func (b *BoardService) Answer(taskID, blockerID string, option int, text string, wake bool) (AnswerResult, error) {
	if option < -1 {
		return AnswerResult{}, fmt.Errorf("%w: option is an index or -1", coord.ErrInvalid)
	}
	ctx := context.Background()
	taskID, blockerID = strings.TrimSpace(taskID), strings.TrimSpace(blockerID)
	if blockerID == NoteItem {
		return b.answerInput(ctx, taskID, option, text, wake)
	}
	t, ok := b.coord.GetTask(taskID)
	if !ok {
		return AnswerResult{}, fmt.Errorf("%w: task %q", coord.ErrNotFound, taskID)
	}
	i := slices.IndexFunc(t.Blockers, func(bl coord.Blocker) bool { return bl.ID == blockerID })
	if i < 0 {
		return AnswerResult{}, fmt.Errorf("%w: task %s has no blocker %q", coord.ErrNotFound, taskID, blockerID)
	}
	bl := t.Blockers[i]
	a := coord.BlockerAnswer{Pick: option + 1, Text: text, Wake: wake, Resolve: b.nobodyCanAct(t, bl.RaisedBy)}
	if a.Resolve && a.Pick == 0 && cmp.Or(bl.Status, coord.BlockerOpen) != coord.BlockerOpen {
		// The person answered before: resolving keeps that answer and
		// takes text as a note, so these words go in as the answer first.
		if _, _, err := b.coord.Answer(ctx, b.user.Key, taskID, blockerID, coord.BlockerAnswer{Text: text, Wake: wake}); err != nil {
			return AnswerResult{}, err
		}
		a.Text = ""
	}
	t, n, err := b.coord.Answer(ctx, b.user.Key, taskID, blockerID, a)
	if err != nil {
		return AnswerResult{}, err
	}
	told := b.tell(ctx, n, TellNextTurn)
	d, err := b.Card(t.ID)
	if err != nil {
		return AnswerResult{}, err
	}
	return AnswerResult{Card: d, Told: told}, nil
}

// nobodyCanAct reports whether no agent will act on an answer to an item
// raiser raised: raiser isn't an agent (the person or Irori, whatever the
// holders, as the answer would go back to them), or is a session the
// roster has as gone and no holder of t is live. The person's answer then
// resolves it, as nobody would resolve or reply.
func (b *BoardService) nobodyCanAct(t coord.Task, raiser string) bool {
	p := b.people()
	if r := p.ref(raiser); raiser == "" || r.Status == "person" || r.Status == "irori" {
		return true
	}
	live := func(key string) bool {
		r := p.ref(key)
		return r.Live && r.Status != "person" && r.Status != "irori"
	}
	return !live(raiser) && !slices.ContainsFunc(t.Holders, live)
}

// ResolveBlocker is the person's "Mark resolved" on an item they answered,
// for when its raiser won't resolve it (it ended after the answer): their
// latest answer stands and note, which may be empty, goes beside the
// check. On an item nobody answered, note is the answer. The holders hear
// at their next turn.
func (b *BoardService) ResolveBlocker(taskID, blockerID, note string) (AnswerResult, error) {
	ctx := context.Background()
	taskID, blockerID = strings.TrimSpace(taskID), strings.TrimSpace(blockerID)
	if blockerID == NoteItem {
		return AnswerResult{}, fmt.Errorf("%w: answer the card's question instead", coord.ErrInvalid)
	}
	t, n, err := b.coord.Answer(ctx, b.user.Key, taskID, blockerID, coord.BlockerAnswer{Text: note, Resolve: true})
	if err != nil {
		return AnswerResult{}, err
	}
	told := b.tell(ctx, n, TellNextTurn)
	d, err := b.Card(t.ID)
	if err != nil {
		return AnswerResult{}, err
	}
	return AnswerResult{Card: d, Told: told}, nil
}

// answerInput answers an input_required card's note: the answer is the
// person's comment to its holders (a wake when wake), and the card goes
// back to working (submitted when nobody holds it).
func (b *BoardService) answerInput(ctx context.Context, id string, option int, text string, wake bool) (AnswerResult, error) {
	t, ok := b.coord.GetTask(id)
	switch {
	case !ok:
		return AnswerResult{}, fmt.Errorf("%w: task %q", coord.ErrNotFound, id)
	case t.State != coord.InputRequired:
		return AnswerResult{}, fmt.Errorf("%w: task %s is %s, not waiting on your input", coord.ErrBadState, id, t.State)
	case option != -1:
		return AnswerResult{}, fmt.Errorf("%w: this question has no recommended answers; answer in your own words", coord.ErrInvalid)
	}
	_, n, err := b.coord.AddComment(ctx, b.user.Key, id, text, wake)
	if err != nil {
		return AnswerResult{}, err
	}
	told := b.tell(ctx, n, TellNextTurn)
	back := coord.Submitted
	if len(t.Holders) > len(t.Assigned) {
		back = coord.Working
	}
	if _, err := b.coord.SetTaskState(ctx, b.user.Key, id, back, ""); err != nil {
		return AnswerResult{}, err
	}
	d, err := b.Card(id)
	if err != nil {
		return AnswerResult{}, err
	}
	return AnswerResult{Card: d, Told: told}, nil
}

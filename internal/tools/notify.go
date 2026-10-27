package tools

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/notify"
)

// The notify tool's limits.
const (
	NotifyLimit    = 5                // cards per agent per NotifyWindow
	NotifyWindow   = 10 * time.Minute // sliding
	NotifyAnswer   = 5 * time.Minute  // how long a card with buttons waits for an answer
	notifyInfoLife = 12 * time.Second // a card without buttons
	maxNotifyText  = 500              // runes
	maxNotifyTitle = 80               // runes
	maxActionLabel = 24               // runes
)

// notifyPerson is how the answer names who was asked: the person at this
// device has no name in Irori beyond "you".
const notifyPerson = "the person"

// notifyState is the notify tool's per-agent rate window and its card ids.
type notifyState struct {
	mu     sync.Mutex
	posted map[string][]time.Time // caller key -> recent card times
	seq    uint64
	answer time.Duration // NotifyAnswer; tests shorten it
}

type notifyArgs struct {
	Text    string   `json:"text"`
	Title   string   `json:"title"`
	Level   string   `json:"level"`
	Actions []string `json:"actions"`
}

// notify posts a card to the person at this device for the caller. With
// actions, the person's answer (a button, a dismiss, or no answer by
// NotifyAnswer) goes back to the caller as a next_turn notice from Irori.
func (s *set) notify(ctx context.Context, c Caller, raw json.RawMessage) string {
	var a notifyArgs
	if err := decode(raw, &a); err != nil {
		return badArgs(err)
	}
	text, title := strings.TrimSpace(a.Text), oneLine(a.Title, maxNotifyTitle+1)
	switch {
	case text == "":
		return errText("give text: what the person must see or decide.")
	case utf8.RuneCountInString(text) > maxNotifyText:
		return errText(fmt.Sprintf("text is %d characters; a card holds at most %d. Say less, and put detail in a message or on the Board.", utf8.RuneCountInString(text), maxNotifyText))
	case utf8.RuneCountInString(title) > maxNotifyTitle:
		return errText(fmt.Sprintf("title is at most %d characters.", maxNotifyTitle))
	}
	level, ok := notify.ParseLevel(a.Level)
	if !ok {
		return errText("level is info, success, warn or error.")
	}
	if len(a.Actions) > notify.MaxActions {
		return errText(fmt.Sprintf("at most %d actions (button labels).", notify.MaxActions))
	}
	var labels []string
	for _, l := range a.Actions {
		l = oneLine(l, maxActionLabel+1)
		switch {
		case l == "":
			return errText("an action label is empty.")
		case utf8.RuneCountInString(l) > maxActionLabel:
			return errText(fmt.Sprintf("an action label is at most %d characters: one or two words.", maxActionLabel))
		}
		labels = append(labels, l)
	}

	n := s.b.Notifier()
	switch {
	case n.Headless():
		return errText("no desktop app runs on this device (Irori is headless), so there is nobody to show a card to. Send a message or use the Board.")
	case !n.Enabled():
		return errText("the person has notifications turned off. Send a message or use the Board instead.")
	}

	st := &s.notifyState
	now := s.now()
	key := c.Sender.Key
	if !st.reserve(key, now) {
		return errText(fmt.Sprintf("you have shown the person %d notification cards in the last %d minutes, which is the limit. "+
			"Only notify for what they must see or decide now; put the rest in a message or on the Board.", NotifyLimit, int(NotifyWindow/time.Minute)))
	}
	// The slot is taken from here on; a card that isn't shown gives it back.
	shown := false
	defer func() {
		if !shown {
			st.unreserve(key, now)
		}
	}()
	st.mu.Lock()
	st.seq++
	id := notify.AskID(strconv.FormatInt(now.UnixNano(), 36) + "-" + strconv.FormatUint(st.seq, 36))
	wait := st.answer
	if wait <= 0 {
		wait = NotifyAnswer
	}
	st.mu.Unlock()

	who := strings.TrimSpace(c.Sender.Name)
	if who == "" {
		who = s.roster().who(c.Sender.Key)
	}
	cardTitle := who
	if title != "" {
		cardTitle = who + ": " + title
	}
	note := notify.Note{ID: id, Title: cardTitle, Body: text, Level: level}
	if len(labels) == 0 {
		note.Expire = notifyInfoLife
	} else {
		asked := title
		if asked == "" {
			asked = oneLine(text, 80)
		}
		var once sync.Once
		answer := func(how string) {
			once.Do(func() { s.notifyAnswer(c.Sender, id, "You asked "+notifyPerson+": "+asked+". "+how) })
		}
		for _, l := range labels {
			note.Actions = append(note.Actions, notify.Action{Label: l, Run: func() { answer("They chose: " + l) }})
		}
		note.OnDismiss = func() { answer("They dismissed it without choosing.") }
		note.OnExpire = func() {
			answer(fmt.Sprintf("No answer: the card went away after %s without a choice.", humanWait(wait)))
		}
		note.Expire = wait
		// The answer may come after the caller has gone; record it now
		// so its key still resolves then.
		if ib := s.b.Inbox(); ib != nil {
			if err := ib.RecordRoster(ctx); err != nil {
				return mapErr(err)
			}
		}
	}
	if !n.Post(note) {
		return errText("the card couldn't be shown (notifications may have just been turned off).")
	}
	shown = true

	if len(labels) == 0 {
		return "Showed the person a card (" + id + "). No answer comes back for a card without actions."
	}
	return "Showed the person a card (" + id + ") with " + strings.Join(quotedAll(labels), " and ") +
		". Their answer (the button, a dismiss, or no answer within " + humanWait(wait) +
		") reaches you at your next turn as a message from Irori; carry on meanwhile and don't ask again."
}

// reserve takes one of key's NotifyLimit slots in the window ending at now,
// or reports that none is free. Checking and taking happen under one lock,
// so calls made at the same time can't all pass the check.
func (st *notifyState) reserve(key string, now time.Time) bool {
	st.mu.Lock()
	defer st.mu.Unlock()
	var kept []time.Time
	for _, t := range st.posted[key] {
		if now.Sub(t) < NotifyWindow {
			kept = append(kept, t)
		}
	}
	if len(kept) >= NotifyLimit {
		if st.posted != nil {
			st.posted[key] = kept
		}
		return false
	}
	if st.posted == nil {
		st.posted = map[string][]time.Time{}
	}
	st.posted[key] = append(kept, now)
	return true
}

// unreserve gives back a slot reserve took at now (its card wasn't shown).
func (st *notifyState) unreserve(key string, now time.Time) {
	st.mu.Lock()
	defer st.mu.Unlock()
	ts := st.posted[key]
	for i := len(ts) - 1; i >= 0; i-- {
		if ts[i].Equal(now) {
			st.posted[key] = append(ts[:i:i], ts[i+1:]...)
			return
		}
	}
}

// notifyAnswer tells the agent that asked what the person did, at its next
// turn, from Irori itself.
func (s *set) notifyAnswer(to inbox.Sender, id, text string) {
	ib := s.b.Inbox()
	if ib == nil {
		return
	}
	_, _, err := ib.SendWith(context.Background(), inbox.SystemSender(),
		inbox.Address{Kind: inbox.ToAgent, Value: to.Key}, text,
		inbox.SendOptions{Mode: inbox.ModeNextTurn, Notice: true, Reason: "notify answer"})
	if err != nil {
		log.Printf("irori: notify %s: answer to %s: %v", id, to.Key, err)
	}
}

func quotedAll(ls []string) []string {
	out := make([]string, len(ls))
	for i, l := range ls {
		out[i] = strconv.Quote(l)
	}
	return out
}

func humanWait(d time.Duration) string {
	if d >= time.Minute && d%time.Minute == 0 {
		m := int(d / time.Minute)
		if m == 1 {
			return "1 minute"
		}
		return strconv.Itoa(m) + " minutes"
	}
	return d.Round(time.Second).String()
}

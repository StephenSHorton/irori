package main

import (
	"sync"

	"github.com/StephenSHorton/irori/internal/notify"
	"github.com/StephenSHorton/irori/internal/update"
)

// updateCardID is the update notification's card id for version v.
func updateCardID(v string) string { return "update:" + v }

// updateCard posts the desktop card that says a downloaded, verified update
// is ready, the way Toru does: sticky, with Install (restart to update now)
// and Later (close the card; the update installs when Irori quits). It is
// GUI only: setupUpdater makes one with the GUI Core's Notifier, so a
// headless Core never gets here, and Notifier.Post already skips the card
// when the person has notifications off.
//
// The card goes up once per version per run. It comes down when the install
// starts (Install here, the banner's Restart now, the tray item) and when a
// different version replaces it.
type updateCard struct {
	n       *notify.Notifier
	install func() error // UpdateService.RestartToUpdate
	open    func()       // shows the window (a click on the card body)

	mu        sync.Mutex
	posted    string // version whose card is up, "" when none
	announced string // last version a card went up for
}

// onStatus follows the updater (UpdateService.SetOnChange).
func (c *updateCard) onStatus(st update.Status) {
	c.mu.Lock()
	var drop string
	if c.posted != "" && (st.Phase != update.PhaseReady || st.Version != c.posted) {
		drop, c.posted = c.posted, ""
	}
	post := st.Phase == update.PhaseReady && st.Version != "" && st.Version != c.announced
	c.mu.Unlock()

	if drop != "" {
		c.n.Clear(updateCardID(drop))
	}
	if post {
		c.post(st.Version, "", false)
	}
}

// post puts the card for version v up. failure, when set, replaces the body
// with why the last Install did not start; silent skips the sound (a reply to
// a click needs no second chime).
func (c *updateCard) post(v, failure string, silent bool) {
	body := "Install restarts Irori. Or keep working: it installs when you quit."
	level := notify.Info
	if failure != "" {
		body = "Couldn't start the installer: " + failure + ". Try again, or quit Irori to install."
		level = notify.Warn
	}
	note := notify.Note{
		ID:     updateCardID(v),
		Title:  "Irori v" + v + " is ready",
		Body:   body,
		Level:  level,
		Sticky: true,
		Silent: silent,
		Actions: []notify.Action{
			{Label: "Install", Run: func() { c.installNow(v) }},
			{Label: "Later"}, // just closes the card; it still installs on quit
		},
		OnClick: c.open,
	}
	if !c.n.Post(note) {
		return
	}
	c.mu.Lock()
	c.posted, c.announced = v, v
	c.mu.Unlock()
}

// installNow is the card's Install: restart to update. RestartToUpdate quits
// Irori on success. On failure the updater is back at ready and nothing else
// would say so, so the card comes back with the reason.
func (c *updateCard) installNow(v string) {
	if c.install == nil {
		return
	}
	if err := c.install(); err != nil {
		c.post(v, err.Error(), true)
	}
}

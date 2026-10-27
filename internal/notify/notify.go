// Package notify is Irori's desktop notifications (docs/design/waits-signals.md
// "Notifications", decision 26): native cards from go-toast that sit above
// every other window, never take focus, and carry up to two buttons.
//
// A Notifier is made once per Core. The desktop app's Notifier shows cards;
// a headless Core's (serve --headless) never does: Post reports false and
// nothing reaches the screen. Every card has an Irori id ("held:<deliveryId>",
// "needs:<taskId>", "msg:<deliveryId>", "ask:<id>", "test"), so the same thing
// never stacks twice (a second Post replaces the card in place) and Clear
// takes it down once the thing is resolved somewhere else.
//
// Watch (watch.go) turns inbox and coord change streams into cards; the
// notify tool (internal/tools) posts a card for an agent and routes the
// answer back to it.
package notify

import (
	"os"
	"strings"
	"sync"
	"time"

	toast "github.com/StephenSHorton/go-toast"
)

// Level picks a card's accent, icon and sound.
type Level string

const (
	Info    Level = "info"
	Success Level = "success"
	Warn    Level = "warn"
	Error   Level = "error"
)

// ParseLevel maps "" to Info and rejects anything that isn't a Level.
func ParseLevel(s string) (Level, bool) {
	switch l := Level(strings.ToLower(strings.TrimSpace(s))); l {
	case "":
		return Info, true
	case Info, Success, Warn, Error:
		return l, true
	}
	return "", false
}

func (l Level) kind() toast.Kind {
	switch l {
	case Success:
		return toast.Success
	case Warn:
		return toast.Warn
	case Error:
		return toast.Error
	}
	return toast.Info
}

// MaxActions is the most buttons a card shows (go-toast's limit).
const MaxActions = 2

// Action is a button. Run is called on its own goroutine after the card
// closes; nil just closes the card.
type Action struct {
	Label string
	Run   func()
}

// Note is one card.
type Note struct {
	// ID is the Irori id (see the package doc). Required: Post refuses a
	// note without one, so nothing ever stacks twice.
	ID    string
	Title string
	Body  string // up to two lines on the card
	Level Level
	// Silent skips the sound.
	Silent bool
	// Sticky cards stay until answered, dismissed or cleared.
	Sticky bool
	// Expire is how long the card lives. With OnExpire set, the Notifier
	// owns the clock: when Expire passes with no button, body click or
	// dismiss, the card is closed and OnExpire runs (go-toast itself has no
	// expiry callback). Zero is go-toast's default lifetime and no OnExpire.
	Expire    time.Duration
	Actions   []Action // at most MaxActions; the first is the primary
	OnClick   func()   // a click on the card body; nil makes a body click a dismiss
	OnDismiss func()   // the person dismissed it (the x, right click)
	OnExpire  func()   // see Expire
}

// Target is where a card's Open takes the person: a tab of the Irori
// window, and optionally a ref to select there ("task:<id>", a delivery id).
type Target struct {
	Tab string `json:"tab"` // "roster", "inbox", "board", ...
	Ref string `json:"ref,omitempty"`
}

// Corner values for Options.Corner.
const (
	CornerTopRight    = toast.CornerTopRight
	CornerBottomRight = toast.CornerBottomRight
	CornerTopLeft     = toast.CornerTopLeft
	CornerBottomLeft  = toast.CornerBottomLeft
)

// Options are the person's notification settings, kept in settings.json in
// Irori's data folder.
type Options struct {
	Enabled bool    `json:"enabled"`
	Sound   bool    `json:"sound"`
	Volume  float64 `json:"volume"` // 0..1
	Corner  string  `json:"corner"` // one of the Corner* values
}

// DefaultOptions is on, with sound, top-right.
func DefaultOptions() Options { return fromToast(toast.DefaultOptions()) }

func (o Options) toToast() toast.Options {
	return toast.Options{Enabled: o.Enabled, Sound: o.Sound, Volume: o.Volume, Corner: o.Corner}
}

func fromToast(o toast.Options) Options {
	return Options{Enabled: o.Enabled, Sound: o.Sound, Volume: o.Volume, Corner: o.Corner}
}

// normalized snaps hand-edited values the way go-toast does.
func (o Options) normalized() Options {
	switch {
	case o.Volume != o.Volume: // NaN
		o.Volume = DefaultOptions().Volume
	case o.Volume < 0:
		o.Volume = 0
	case o.Volume > 1:
		o.Volume = 1
	}
	switch o.Corner {
	case CornerTopRight, CornerBottomRight, CornerTopLeft, CornerBottomLeft:
	default:
		o.Corner = CornerTopRight
	}
	return o
}

// Toaster is the card system: go-toast on the desktop, a fake in tests.
type Toaster interface {
	Configure(toast.Config)
	SetOptions(toast.Options)
	Post(toast.Note)
	Close(id string)
}

// goToast is the real Toaster.
type goToast struct{}

func (goToast) Configure(c toast.Config)   { toast.Configure(c) }
func (goToast) SetOptions(o toast.Options) { toast.SetOptions(o) }
func (goToast) Post(n toast.Note)          { toast.Post(n) }
func (goToast) Close(id string)            { toast.Close(id) }

// CaptureEnv set to 1 lets screenshots and recordings see the cards
// (go-toast hides them by default with WDA_EXCLUDEFROMCAPTURE). It is for
// checking a card's look with a screen capture.
const CaptureEnv = "IRORI_TOAST_CAPTURE"

// Config configures New.
type Config struct {
	// Dir is Irori's data folder; settings.json there holds Options. ""
	// keeps them in memory only.
	Dir string
	// Headless: never show a card (serve --headless).
	Headless bool
	// Toaster defaults to go-toast. Tests pass a fake.
	Toaster Toaster
	// CaptureVisible turns go-toast's CaptureExclude off. New also turns it
	// off when CaptureEnv is "1".
	CaptureVisible bool
}

// Notifier shows Irori's cards. Safe for concurrent use. A nil *Notifier
// is a headless one.
type Notifier struct {
	t        Toaster // nil when headless
	dir      string
	mu       sync.Mutex
	opts     Options
	live     map[string]*entry // cards Post showed that haven't closed through us
	gen      uint64
	opener   func(Target)
	onChange []func(Options)
}

// entry is one posted card's bookkeeping: gen tells a replaced card's
// callbacks from the live one's, so only one of a card's outcomes ever runs.
type entry struct {
	gen   uint64
	timer *time.Timer
}

// New loads the saved Options and, unless headless, configures go-toast:
// AppName "Irori", the Light palette, 10 dip corners, hidden from screen
// capture (see CaptureEnv).
func New(c Config) *Notifier {
	n := &Notifier{dir: c.Dir, live: map[string]*entry{}}
	n.opts = load(c.Dir)
	if c.Headless {
		return n
	}
	n.t = c.Toaster
	if n.t == nil {
		n.t = goToast{}
	}
	tc := toast.DefaultConfig()
	tc.AppName = "Irori"
	tc.Palette = toast.Light
	tc.CornerRadius = 10
	tc.CaptureExclude = !(c.CaptureVisible || os.Getenv(CaptureEnv) == "1")
	n.t.Configure(tc)
	n.t.SetOptions(n.opts.toToast())
	return n
}

// Headless reports whether this Notifier never shows cards.
func (n *Notifier) Headless() bool { return n == nil || n.t == nil }

// Enabled reports whether a Post would show a card now: not headless and
// the person has notifications on.
func (n *Notifier) Enabled() bool {
	if n.Headless() {
		return false
	}
	n.mu.Lock()
	defer n.mu.Unlock()
	return n.opts.Enabled
}

// Options returns the person's settings.
func (n *Notifier) Options() Options {
	if n == nil {
		return DefaultOptions()
	}
	n.mu.Lock()
	defer n.mu.Unlock()
	return n.opts
}

// SetOptions applies and saves the person's settings. Turning
// notifications off takes every card down (their OnExpire still runs when
// due, so an agent waiting on an answer hears that none came).
func (n *Notifier) SetOptions(o Options) (Options, error) {
	if n == nil {
		return DefaultOptions(), nil
	}
	o = o.normalized()
	n.mu.Lock()
	n.opts = o
	fns := append([]func(Options){}, n.onChange...)
	n.mu.Unlock()
	if n.t != nil {
		n.t.SetOptions(o.toToast())
	}
	err := save(n.dir, o)
	for _, fn := range fns {
		fn(o)
	}
	return o, err
}

// OnOptions registers fn for every SetOptions (the GUI announces them).
func (n *Notifier) OnOptions(fn func(Options)) {
	if n == nil {
		return
	}
	n.mu.Lock()
	n.onChange = append(n.onChange, fn)
	n.mu.Unlock()
}

// SetOpener sets what Open does: the desktop app shows its window at the
// target's tab. Headless leaves it unset.
func (n *Notifier) SetOpener(fn func(Target)) {
	if n == nil {
		return
	}
	n.mu.Lock()
	n.opener = fn
	n.mu.Unlock()
}

// Open shows the Irori window at t (a no-op with no opener).
func (n *Notifier) Open(t Target) {
	if n == nil {
		return
	}
	n.mu.Lock()
	fn := n.opener
	n.mu.Unlock()
	if fn != nil {
		fn(t)
	}
}

// Post shows note, replacing the live card with the same ID. It reports
// whether a card went up: false when headless, when notifications are off,
// or when the note has no ID or nothing to say.
func (n *Notifier) Post(note Note) bool {
	note.ID = strings.TrimSpace(note.ID)
	if note.ID == "" || strings.TrimSpace(note.Title+note.Body) == "" || !n.Enabled() {
		return false
	}
	n.mu.Lock()
	n.gen++
	gen := n.gen
	if old := n.live[note.ID]; old != nil && old.timer != nil {
		old.timer.Stop()
	}
	e := &entry{gen: gen}
	n.live[note.ID] = e
	if note.OnExpire != nil && note.Expire > 0 {
		id, fn := note.ID, note.OnExpire
		e.timer = time.AfterFunc(note.Expire, func() {
			if n.take(id, gen) {
				n.t.Close(id)
				fn()
			}
		})
	}
	n.mu.Unlock()

	// Each outcome runs at most once, and only for the card that is live.
	wrap := func(fn func()) func() {
		if fn == nil {
			return func() { n.take(note.ID, gen) }
		}
		return func() {
			if n.take(note.ID, gen) {
				fn()
			}
		}
	}
	tn := toast.Note{
		ID: note.ID, Title: note.Title, Body: note.Body, Kind: note.Level.kind(),
		Silent: note.Silent, Sticky: note.Sticky, Expire: note.Expire,
		OnDismiss: wrap(note.OnDismiss),
	}
	if note.OnExpire != nil && note.Expire > 0 {
		// The Notifier's timer closes the card; go-toast keeps it until then.
		tn.Sticky, tn.Expire = true, 0
	}
	if note.OnClick != nil {
		tn.OnClick = wrap(note.OnClick)
	}
	for i, a := range note.Actions {
		if i == MaxActions {
			break
		}
		tn.Actions = append(tn.Actions, toast.Action{Label: a.Label, Run: wrap(a.Run)})
	}
	n.t.Post(tn)
	return true
}

// take ends the card id's generation gen, reporting whether it was still
// live (so its outcome should run).
func (n *Notifier) take(id string, gen uint64) bool {
	n.mu.Lock()
	defer n.mu.Unlock()
	e := n.live[id]
	if e == nil || e.gen != gen {
		return false
	}
	if e.timer != nil {
		e.timer.Stop()
	}
	delete(n.live, id)
	return true
}

// Clear takes the card id down without running any of its callbacks: the
// thing it was about was resolved elsewhere (released, dismissed, answered
// in the app). A no-op when no such card is up.
func (n *Notifier) Clear(id string) {
	if n.Headless() || id == "" {
		return
	}
	n.mu.Lock()
	e := n.live[id]
	if e != nil {
		if e.timer != nil {
			e.timer.Stop()
		}
		delete(n.live, id)
	}
	n.mu.Unlock()
	if e != nil {
		n.t.Close(id)
	}
}

// Live reports whether the card id is up as far as the Notifier knows (it
// was posted and hasn't been answered, dismissed, expired or cleared).
// Cards go-toast let expire on its own lifetime stay "live" here until the
// next Post or Clear of that id; that only costs a no-op Close.
func (n *Notifier) Live(id string) bool {
	if n == nil {
		return false
	}
	n.mu.Lock()
	defer n.mu.Unlock()
	return n.live[id] != nil
}

// Card ids. Each thing that can need the person has one id, so it never
// stacks twice.
func HeldID(deliveryID string) string    { return "held:" + deliveryID }
func NeedsID(taskID string) string       { return "needs:" + taskID }
func MessageID(deliveryID string) string { return "msg:" + deliveryID }
func AskID(id string) string             { return "ask:" + id }

// TestID is the settings' "Send a test notification" card.
const TestID = "test"

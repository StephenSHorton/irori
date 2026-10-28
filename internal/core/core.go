// Package core wires Irori's services together: device identity, the event
// log, presence and the loopback API. The GUI (`irori`) and the headless node
// (`irori serve --headless`) both run exactly one Core.
package core

import (
	"context"
	"errors"
	"fmt"
	"log"
	"net"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/api"
	"github.com/StephenSHorton/irori/internal/ask"
	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/mesh/roomnet"
	"github.com/StephenSHorton/irori/internal/notify"
	"github.com/StephenSHorton/irori/internal/pair"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/repo"
	"github.com/StephenSHorton/irori/internal/roster"
	"github.com/StephenSHorton/irori/internal/scrub"
	"github.com/StephenSHorton/irori/internal/store"
	"github.com/StephenSHorton/irori/internal/tools"
)

// ErrAlreadyRunning means another Irori on this device holds the lockfile.
var ErrAlreadyRunning = errors.New("irori is already running on this device")

// Options configure Start.
type Options struct {
	DataDir   string
	ClaudeDir string
	Version   string
	Headless  bool
	// Keystore seals the person and device keys; nil is the OS default
	// (DPAPI on Windows, the Keychain on macOS). Tests pass a file keystore.
	Keystore identity.Keystore
	// Net is the transport the mesh runs on once the person has a second
	// device (v0.4 §4.5); nil is go-room on the public brokers, with the
	// relay from relay.json. Tests pass a mesh.FakeNet.
	Net mesh.Transport
	// NetIntervals shortens the mesh's timers (tests).
	NetIntervals mesh.Intervals
	// Now is the wall clock of the log's HLC, the future-row hold, the mesh
	// and the asks; nil is time.Now. Tests set a device's clock off.
	Now func() time.Time
}

// Core is one running Irori.
type Core struct {
	opts     Options
	device   device.Identity
	ident    *identity.Identity
	chain    *identity.Chain
	store    *store.Store
	presence *presence.Watcher
	repos    *repo.Resolver
	inbox    *inbox.Inbox
	coord    *coord.Coord
	tools    *tools.Set
	notifier *notify.Notifier
	members  *members
	capture  *capture.Capture
	mesh     *mesh.Node
	pair     *pair.Pairer
	roster   *roster.Roster
	asks     *ask.Book
	files    *blob.Files // attachments (v0.4 §6.6)
	netClose func()      // stops the default transport's relay; nil for a passed-in one
	net      mesh.Transport
	scrubber *scrub.Scrubber
	secretMu sync.Mutex
	secrets  []string // exact values capture redacts: the lockfile token, member secrets
	reviveMu sync.Mutex
	reviver  func(sessionID string) (string, error) // the GUI's Revive (SetReviver); nil headless
	cancel   context.CancelFunc
	done     chan struct{} // closed when the service goroutines finish; nil before they start
}

// Start brings everything up and writes the lockfile. Close undoes it.
func Start(parent context.Context, opts Options) (*Core, error) {
	if err := os.MkdirAll(opts.DataDir, 0o700); err != nil {
		return nil, err
	}
	if lf, err := discovery.Read(opts.DataDir); err == nil && lf.PID != os.Getpid() {
		return nil, fmt.Errorf("%w (pid %d)", ErrAlreadyRunning, lf.PID)
	}
	if err := resetIfAsked(opts.DataDir); err != nil {
		return nil, err
	}
	id, err := device.LoadOrCreate(opts.DataDir)
	if err != nil {
		return nil, err
	}
	// v0.4 identity (design §2): the device key signs every row; the person
	// and its cert chain decide which "user:" keys are the owner's.
	ident, err := identity.Load(opts.DataDir, id, opts.Keystore)
	if err != nil {
		return nil, fmt.Errorf("identity: %w", err)
	}
	id = ident.Device
	chain := identity.NewChain(ident.Person.ID, ident.Person.RootDeviceID, id.ID)
	ctx, cancel := context.WithCancel(parent)
	// OpenWith runs the one-time v0.4 migration on a v0.3 log (irori.db is
	// copied to irori.db.v0.3 first) before anything else reads it.
	st, err := store.OpenWith(ctx, filepath.Join(opts.DataDir, "irori.db"), id.ID, hlc.New(opts.Now),
		store.OpenOptions{Signer: ident, Verifier: chain, Now: opts.Now})
	if err != nil {
		cancel()
		return nil, fmt.Errorf("open event log: %w", err)
	}
	role := identity.RoleDesktop
	if opts.Headless {
		role = identity.RoleNode
	}
	if err := ident.EnsureSelfCert(ctx, st, chain, role); err != nil {
		st.Close()
		cancel()
		return nil, fmt.Errorf("identity: %w", err)
	}
	// A device that paired but stopped before its first sync learns the
	// person rows it was handed again (pair.RowsFile).
	if err := pair.LearnSaved(ctx, opts.DataDir, st, chain); err != nil {
		log.Printf("pairing rows: %v", err)
	}
	repos := repo.New()
	c := &Core{
		opts:   opts,
		device: id,
		ident:  ident,
		chain:  chain,
		store:  st,
		presence: presence.New(presence.Options{
			ClaudeDir: opts.ClaudeDir, DeviceID: id.ID, RepoKey: repos.Key,
		}),
		repos:    repos,
		members:  newMembers(id.ID),
		scrubber: scrub.New(),
		cancel:   cancel,
	}
	if _, err := st.Append(ctx, "irori", store.KindDeviceStarted, map[string]any{
		"name": id.Name, "os": id.OS, "version": opts.Version, "headless": opts.Headless,
	}); err != nil {
		c.Close()
		return nil, err
	}
	// One roster across devices (v0.4 §6.2): this device's presence plus
	// every other device's agent rows. The mesh node is built below; its
	// liveness is read per call.
	if c.roster, err = roster.Start(ctx, roster.Options{Store: st, Self: id.ID, Local: c.LocalAgents,
		Liveness: func(dev string) string {
			if c.mesh == nil {
				return roster.Offline
			}
			return c.mesh.Liveness(dev)
		},
		Revoked: func(dev string) bool { _, revoked := chain.Cut(dev); return revoked },
	}); err != nil {
		c.Close()
		return nil, fmt.Errorf("roster: %w", err)
	}
	// The inbox is built before capture: PROwners reads c.capture per call.
	prOwners := func(prKey string) []string {
		if c.capture == nil {
			return nil
		}
		return c.capture.PROwners(prKey)
	}
	// The inbox routes against the mesh roster: a message can go to an
	// agent on another device, and the bypass rule reads the sender's card
	// wherever it runs.
	if c.inbox, err = inbox.New(ctx, st, meshView{c}, inbox.Options{DeviceID: id.ID, PROwners: prOwners, IsOwner: chain.IsOwner}); err != nil {
		c.Close()
		return nil, fmt.Errorf("inbox: %w", err)
	}
	if c.coord, err = coord.New(ctx, st, c, coord.Options{IsOwner: chain.IsOwner, Now: opts.Now,
		Revoked: func(dev string) bool { _, revoked := chain.Cut(dev); return revoked }}); err != nil {
		c.Close()
		return nil, fmt.Errorf("coord: %w", err)
	}
	if c.capture, err = capture.New(ctx, st, c, capture.Options{
		ClaudeDir: opts.ClaudeDir, DataDir: opts.DataDir, DeviceID: id.ID,
		RepoKey: repos.Key, Scrub: c.scrubber,
	}); err != nil {
		c.Close()
		return nil, fmt.Errorf("capture: %w", err)
	}
	// Search and show reach the Board and the inbox through capture.
	c.capture.AddSource(c.coord)
	c.capture.AddSource(c.inbox)
	// Cards are the desktop app's; a headless Core's Notifier never shows one.
	c.notifier = notify.New(notify.Config{Dir: opts.DataDir, Headless: opts.Headless})
	// Approvals as rows (v0.4 §6.7): every desktop shows them, the asking
	// device decides, and a revive from another device waits for one.
	if c.asks, err = ask.Start(ctx, ask.Options{Store: st, Self: id.ID, IsOwner: chain.IsOwner, Now: opts.Now,
		Revive: c.reviveForAsk, Name: c.agentName}); err != nil {
		c.Close()
		return nil, fmt.Errorf("asks: %w", err)
	}
	c.tools = tools.New(meshView{c})
	if c.files, err = blob.OpenFiles(filepath.Join(opts.DataDir, "files")); err != nil {
		c.Close()
		return nil, fmt.Errorf("attachments: %w", err)
	}

	// The net layer (v0.4 §4.5) is built here but opens nothing until the
	// person has a second device: a device on its own makes no broker
	// connection and binds no UDP socket.
	tr := opts.Net
	if tr == nil {
		relay, err := mesh.LoadRelay(opts.DataDir)
		if err != nil {
			log.Printf("relay setting: %v (using Direct only)", err)
		}
		rt := roomnet.New(roomnet.Options{Self: id.ID, Relay: relay, Logf: log.Printf})
		tr, c.netClose = rt, rt.Close
	}
	c.net = tr
	c.mesh = mesh.New(mesh.Options{Store: st, Ident: ident, Chain: chain, Net: tr, Version: opts.Version,
		Files: c.files, DataDir: opts.DataDir, Intervals: opts.NetIntervals, Now: opts.Now, Ignored: c.ignoredBy, Logf: log.Printf})
	// Pairing (v0.4 §3) opens its own room on the same transport, only
	// while a window is open; GUI and CLI only, never the loopback API.
	c.pair = pair.New(pair.Options{Net: tr, Ident: ident, Chain: chain, Store: st, DataDir: opts.DataDir,
		Kick: c.mesh.Kick, Logf: log.Printf})

	token, err := discovery.NewToken()
	if err != nil {
		c.Close()
		return nil, err
	}
	c.addSecret(token)
	ln, err := api.Listen()
	if err != nil {
		c.Close()
		return nil, err
	}
	srv := api.New(c, token)
	// The API's per-run secret for the person's view (decision D10(a))
	// stays inside it: the GUI reads the person's view through Wails
	// bindings, so nothing hands the secret on, and a bearer token alone
	// never gets that view.
	// Other devices' queued deliveries get this device's own hold decision
	// before the API serves an attach stream that could wake them (§6.3).
	c.inbox.CatchUp(ctx)
	changes := c.presence.Subscribe(ctx)
	c.done = make(chan struct{})
	go func() {
		defer close(c.done)
		go func() {
			if err := c.presence.Run(ctx); err != nil && ctx.Err() == nil {
				log.Printf("presence: %v", err)
			}
		}()
		capDone := make(chan struct{})
		defer func() { <-capDone }() // capture stops appending before the log closes
		go func() {
			defer close(capDone)
			// Backfill runs inside Run, after Start has returned.
			if err := c.capture.Run(ctx); err != nil && ctx.Err() == nil {
				log.Printf("capture: %v", err)
			}
		}()
		meshDone := make(chan struct{})
		defer func() { <-meshDone }() // the mesh stops inserting before the log closes
		go func() {
			defer close(meshDone)
			// Run waits for a second device before it opens anything; any
			// failure leaves a working single-device app.
			if err := c.mesh.Run(ctx); err != nil && ctx.Err() == nil {
				log.Printf("mesh: %v", err)
			}
		}()
		// Other devices' rows fold into the inbox and coord as the mesh
		// inserts them (v0.4 §6.1); both write as owners, so they stop
		// before the log closes.
		followDone := make(chan struct{})
		defer func() { <-followDone }()
		defer func() { <-c.asks.Done() }() // asks resolve as the asking device
		go func() {
			defer close(followDone)
			fd := make(chan struct{})
			go func() { defer close(fd); c.coord.Follow(ctx) }()
			c.inbox.Follow(ctx)
			<-fd
		}()
		go c.record(ctx, changes)
		go c.sweep(ctx)
		if err := srv.Serve(ctx, ln); err != nil && ctx.Err() == nil {
			log.Printf("api: %v", err)
		}
	}()
	if err := discovery.Write(opts.DataDir, discovery.Lockfile{
		PID:       os.Getpid(),
		Port:      ln.Addr().(*net.TCPAddr).Port,
		Token:     token,
		Version:   opts.Version,
		DeviceID:  id.ID,
		Headless:  opts.Headless,
		StartedAt: time.Now().UTC(),
	}); err != nil {
		c.Close()
		return nil, err
	}
	return c, nil
}

// Close stops the services, removes the lockfile if it is still ours and
// closes the log.
func (c *Core) Close() error {
	if c.pair != nil {
		c.pair.Cancel()
	}
	c.cancel()
	if c.done != nil {
		select {
		case <-c.done:
		case <-time.After(5 * time.Second):
		}
	}
	if c.netClose != nil {
		c.netClose()
	}
	_ = discovery.Remove(c.opts.DataDir, os.Getpid())
	return c.store.Close()
}

// record turns roster changes into log rows. Status rows are written only
// when status or name changes, so a busy session doesn't flood the log.
func (c *Core) record(ctx context.Context, changes <-chan presence.Change) {
	last := map[string]presence.AgentCard{}
	for ch := range changes {
		card := ch.Card
		actor := "irori"
		var err error
		switch ch.Kind {
		case presence.Added:
			last[card.Key] = card
			_, err = c.store.Append(ctx, actor, store.KindAgentSeen, c.rowCard(card))
		case presence.Updated:
			prev, ok := last[card.Key]
			last[card.Key] = card
			if ok && prev.Status == card.Status && prev.Name == card.Name {
				continue
			}
			_, err = c.store.Append(ctx, actor, store.KindAgentStatus, map[string]any{
				"key": card.Key, "status": card.Status, "name": card.Name,
			})
		case presence.Removed:
			delete(last, card.Key)
			_, err = c.store.Append(ctx, actor, store.KindAgentGone, map[string]any{
				"key": card.Key, "reason": card.Status,
			})
		}
		if err != nil && ctx.Err() == nil {
			log.Printf("record %s: %v", ch.Kind, err)
		}
	}
}

// rowCard is the agent.seen card as it replicates (v0.4 §2): no transcript
// path or pid (they mean nothing on another machine), no last assistant
// text (decision D9: it stays in this device's memory), and the title
// scrubbed, because a replicated row can't be scrubbed later.
func (c *Core) rowCard(card presence.AgentCard) presence.AgentCard {
	card.LastText, card.Transcript, card.PID, card.DeviceState = "", "", 0, ""
	card.Title = c.scrubber.String(card.Title)
	return card
}

// coordSweepInterval: coord's sweeper starts a vanished holder's grace
// clock only when it runs, so it runs more often than the inbox sweeper.
const coordSweepInterval = 5 * time.Second

// sweep runs the inbox sweeper every inbox.SweepInterval and the coord
// sweeper (SweepNotices, its notices routed: sweepCoord) every
// coordSweepInterval.
func (c *Core) sweep(ctx context.Context) {
	t := time.NewTicker(inbox.SweepInterval)
	defer t.Stop()
	ct := time.NewTicker(coordSweepInterval)
	defer ct.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			if err := c.inbox.Sweep(ctx); err != nil && ctx.Err() == nil {
				log.Printf("inbox sweep: %v", err)
			}
		case <-ct.C:
			if err := c.sweepCoord(ctx); err != nil && ctx.Err() == nil {
				log.Printf("coord sweep: %v", err)
			}
		}
	}
}

// Inbox is the messaging core.
func (c *Core) Inbox() *inbox.Inbox { return c.inbox }

// Coord holds tasks and leases.
func (c *Core) Coord() *coord.Coord { return c.coord }

// UserSender is the person at this device's GUI.
func (c *Core) UserSender() inbox.Sender {
	return inbox.Sender{Key: "user:" + c.device.ID, Name: "you", Kind: "user"}
}

// IsOwner reports whether key is the person's: exactly "user:<deviceId>" for
// this device or a device ever certified to the person (v0.4 §2).
func (c *Core) IsOwner(key string) bool { return c.chain.IsOwner(key) }

// Person is this device's person (v0.4 identity).
func (c *Core) Person() identity.Person { return c.ident.Current() }

// CertifiedDevices lists every device ever certified to the person.
func (c *Core) CertifiedDevices() []identity.DeviceState { return c.chain.Devices() }

// Mesh is the net layer (v0.4 §4, §5). It runs only once the person has a
// second device.
func (c *Core) Mesh() *mesh.Node { return c.mesh }

// Pairing is the pairing flow (v0.4 §3). Only Wails bindings and the
// `irori pair join` CLI reach it; it is not on the loopback API.
func (c *Core) Pairing() *pair.Pairer { return c.pair }

// NetStatus is the mesh's state and every other device, for Devices and
// GET /v1/devices.
func (c *Core) NetStatus() mesh.Status { return c.mesh.Status() }

// Version implements api.Backend.
func (c *Core) Version() string { return c.opts.Version }

// Device implements api.Backend.
func (c *Core) Device() device.Identity { return c.device }

// Agents implements api.Backend: LocalAgents. Attach and the API's caller
// checks only ever concern sessions on this device.
func (c *Core) Agents() []presence.AgentCard { return c.LocalAgents() }

// LocalAgents is this device's roster (v0.4 §6.2): the presence roster plus
// a synthetic card for each bound Grok member (agent "grok") whose stream is
// open or closed less than inbox.ReattachGrace ago. It feeds attach, Revive
// and presence recording.
func (c *Core) LocalAgents() []presence.AgentCard {
	return append(c.presence.Snapshot(), c.members.cards()...)
}

// MeshAgents is the roster across the person's devices (v0.4 §6.2): this
// device's cards, then every other device's from their agent rows, each
// with that device's liveness. It feeds inbox routing, the tools and the
// GUI.
func (c *Core) MeshAgents() []presence.AgentCard {
	if c.roster == nil {
		return c.LocalAgents()
	}
	return c.roster.Agents()
}

// Roster is the mesh roster (MeshAgents), for its change signal.
func (c *Core) Roster() *roster.Roster { return c.roster }

// Asks are the person's approvals as rows (v0.4 §6.7).
func (c *Core) Asks() *ask.Book { return c.asks }

// SetReviver gives the Core the desktop app's Revive (claude --bg --resume),
// which a revive ask answered from any device runs here (v0.4 §6.3). A
// headless Core has none, and such an ask resolves without waking anything.
func (c *Core) SetReviver(fn func(sessionID string) (string, error)) {
	c.reviveMu.Lock()
	defer c.reviveMu.Unlock()
	c.reviver = fn
}

// reviveForAsk revives target, a session on this device, once its revive
// ask was answered for exactly it.
func (c *Core) reviveForAsk(_ context.Context, target string) error {
	c.reviveMu.Lock()
	fn := c.reviver
	c.reviveMu.Unlock()
	if fn == nil {
		return errors.New("no desktop app runs on this device to revive it")
	}
	dev, session, ok := strings.Cut(target, "/")
	if !ok || dev != c.device.ID {
		return fmt.Errorf("%s is not a session on this device", target)
	}
	_, err := fn(session)
	return err
}

// agentName is an agent key's card name in the mesh roster, or "".
func (c *Core) agentName(key string) string {
	for _, a := range c.MeshAgents() {
		if a.Key == key {
			return a.Name
		}
	}
	return ""
}

// meshView is the Core with Agents() as the mesh roster: what the inbox
// routes against and what the tools show.
type meshView struct{ *Core }

func (m meshView) Agents() []presence.AgentCard { return m.MeshAgents() }

// Notifier shows desktop notification cards (none when headless). The GUI
// runs notify.Watch over it and gives it the window opener.
func (c *Core) Notifier() *notify.Notifier { return c.notifier }

// Tools is the agent tool registry served at /v1/tools.
func (c *Core) Tools() *tools.Set { return c.tools }

// Events implements api.Backend.
func (c *Core) Events(ctx context.Context, f store.Filter) ([]store.Event, error) {
	return c.store.List(ctx, f)
}

// SubscribeEvents implements api.Backend: rows appended or replicated after
// the call, with a zero Event as the resync signal (store.Subscribe).
func (c *Core) SubscribeEvents(ctx context.Context) <-chan store.Event {
	return c.store.Subscribe(ctx)
}

// Changes streams roster changes for the UI until ctx ends.
func (c *Core) Changes(ctx context.Context) <-chan presence.Change {
	return c.presence.Subscribe(ctx)
}

var _ api.Backend = (*Core)(nil)

// Capture is the capture service (v0.3).
func (c *Core) Capture() *capture.Capture { return c.capture }

// addSecret gives capture's scrubber one more exact value to redact. Values
// are kept for the Core's life, so a retired member secret that shows up in
// a transcript later is still redacted. Attach secrets are minted inside
// the API's stream handler (api/stream.go) and are not passed here.
func (c *Core) addSecret(v string) {
	if v == "" {
		return
	}
	c.secretMu.Lock()
	defer c.secretMu.Unlock()
	c.secrets = append(c.secrets, v)
	c.scrubber.SetExact(c.secrets...)
}

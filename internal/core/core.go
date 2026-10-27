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
	"time"

	"github.com/StephenSHorton/irori/internal/api"
	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/discovery"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/StephenSHorton/irori/internal/store"
)

// ErrAlreadyRunning means another Irori on this device holds the lockfile.
var ErrAlreadyRunning = errors.New("irori is already running on this device")

// Options configure Start.
type Options struct {
	DataDir   string
	ClaudeDir string
	Version   string
	Headless  bool
}

// Core is one running Irori.
type Core struct {
	opts     Options
	device   device.Identity
	store    *store.Store
	presence *presence.Watcher
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
	id, err := device.LoadOrCreate(opts.DataDir)
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithCancel(parent)
	st, err := store.Open(ctx, filepath.Join(opts.DataDir, "irori.db"), id.ID, hlc.New(nil))
	if err != nil {
		cancel()
		return nil, fmt.Errorf("open event log: %w", err)
	}
	c := &Core{
		opts:     opts,
		device:   id,
		store:    st,
		presence: presence.New(presence.Options{ClaudeDir: opts.ClaudeDir, DeviceID: id.ID}),
		cancel:   cancel,
	}
	if _, err := st.Append(ctx, "irori", store.KindDeviceStarted, map[string]any{
		"name": id.Name, "os": id.OS, "version": opts.Version, "headless": opts.Headless,
	}); err != nil {
		c.Close()
		return nil, err
	}

	token, err := discovery.NewToken()
	if err != nil {
		c.Close()
		return nil, err
	}
	ln, err := api.Listen()
	if err != nil {
		c.Close()
		return nil, err
	}
	srv := api.New(c, token)
	changes := c.presence.Subscribe(ctx)
	c.done = make(chan struct{})
	go func() {
		defer close(c.done)
		go func() {
			if err := c.presence.Run(ctx); err != nil && ctx.Err() == nil {
				log.Printf("presence: %v", err)
			}
		}()
		go c.record(ctx, changes)
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
	c.cancel()
	if c.done != nil {
		select {
		case <-c.done:
		case <-time.After(5 * time.Second):
		}
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
			_, err = c.store.Append(ctx, actor, store.KindAgentSeen, card)
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

// Version implements api.Backend.
func (c *Core) Version() string { return c.opts.Version }

// Device implements api.Backend.
func (c *Core) Device() device.Identity { return c.device }

// Agents implements api.Backend.
func (c *Core) Agents() []presence.AgentCard { return c.presence.Snapshot() }

// Events implements api.Backend.
func (c *Core) Events(ctx context.Context, f store.Filter) ([]store.Event, error) {
	return c.store.List(ctx, f)
}

// Changes streams roster changes for the UI until ctx ends.
func (c *Core) Changes(ctx context.Context) <-chan presence.Change {
	return c.presence.Subscribe(ctx)
}

var _ api.Backend = (*Core)(nil)

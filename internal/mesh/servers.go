package mesh

import (
	"context"
	"encoding/json"
	"net"
	"net/netip"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strconv"
	"sync"

	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/store"
)

// LANPort is the LAN carrier's fixed UDP port on every device (§4.2): a
// device has to know where to send before it has heard anyone.
const LANPort = 38419

// Reach values of a node.addr row (§4.2, §7.1).
const (
	ReachOK       = "ok"       // a mapping is up (or a manual address) and the external address is public
	ReachCGNAT    = "cgnat"    // the mapping is up, but the router's external address is shared
	ReachUnmapped = "unmapped" // the router answered neither UPnP nor NAT-PMP, refused, or mapping is off
	ReachOff      = "off"      // the server was switched off: stop dialing it
)

// NodeAddr is a node.addr row's payload: where a home server can be reached
// (§4.2). Only the server's own device writes it, and only when something
// in it changes.
type NodeAddr struct {
	V      int         `json:"v"`
	Port   int         `json:"port,omitempty"`
	LAN    []string    `json:"lan,omitempty"`
	V6     []string    `json:"v6,omitempty"`
	Public *PublicAddr `json:"public"`
	Via    string      `json:"via"`   // "upnp" | "natpmp" | "manual" | ""
	Reach  string      `json:"reach"` // ReachOK, ReachCGNAT, ReachUnmapped or ReachOff
	At     int64       `json:"at"`    // unix ms; Same ignores it
}

// PublicAddr is the router's external address and the mapped ports, or an
// address the person entered (Host may then be a host name).
type PublicAddr struct {
	IP  string `json:"ip"`
	TCP int    `json:"tcp"`
	UDP int    `json:"udp"`
}

// Same reports whether a and b say the same thing, ignoring when.
func (a NodeAddr) Same(b NodeAddr) bool {
	a.At, b.At = 0, 0
	ra, _ := json.Marshal(a)
	rb, _ := json.Marshal(b)
	return string(ra) == string(rb)
}

// Dial is every host:port the row names, LAN first: at home the LAN address
// wins, so a router without NAT loopback doesn't matter.
func (a NodeAddr) Dial() []string {
	if a.Reach == ReachOff || a.Port == 0 {
		return nil
	}
	var out []string
	port := strconv.Itoa(a.Port)
	for _, ip := range a.LAN {
		out = append(out, net.JoinHostPort(ip, port))
	}
	for _, ip := range a.V6 {
		out = append(out, net.JoinHostPort(ip, port))
	}
	if p := a.Public; p != nil && p.IP != "" && p.TCP > 0 && a.Reach == ReachOK {
		out = append(out, net.JoinHostPort(p.IP, strconv.Itoa(p.TCP)))
	}
	return out
}

// Outside is the address a device away from home dials (and types to join,
// §3), or "" when the server isn't reachable from outside.
func (a NodeAddr) Outside() string {
	if p := a.Public; p != nil && p.IP != "" && p.TCP > 0 && a.Reach == ReachOK {
		return net.JoinHostPort(p.IP, strconv.Itoa(p.TCP))
	}
	return ""
}

// ServersFile keeps, per home server, the last address that completed a
// hub handshake and the addresses a pairing handed this device until the
// server's node.addr row arrives (§4.2). Device-local; no secret in it.
const ServersFile = "hub-addrs.json"

type serverCache struct {
	Last string   `json:"last,omitempty"`
	Hint []string `json:"hint,omitempty"`
}

type serverRow struct {
	seq  int64
	addr NodeAddr
}

// Server is one home server this device knows of and where to dial it.
type Server struct {
	ID    string
	Addrs []string
}

// Servers is the directory of home servers (§4.2): the node.addr fold, the
// LAN address each server's datagrams came from, the last address that
// worked, and pairing hints. roomnet dials what it lists; the home server
// reads its own row back to write only on change.
type Servers struct {
	self  string
	chain *identity.Chain
	dir   string

	mu    sync.Mutex
	rows  map[string]serverRow
	heard map[string]netip.Addr
	cache map[string]serverCache
	watch []func()
}

// NewServers returns an empty directory; Load fills it from the log.
func NewServers(self string, chain *identity.Chain, dataDir string) *Servers {
	s := &Servers{self: self, chain: chain, dir: dataDir, rows: map[string]serverRow{},
		heard: map[string]netip.Addr{}, cache: map[string]serverCache{}}
	if dataDir != "" {
		if raw, err := os.ReadFile(filepath.Join(dataDir, ServersFile)); err == nil {
			_ = json.Unmarshal(raw, &s.cache)
		}
	}
	if s.cache == nil {
		s.cache = map[string]serverCache{}
	}
	return s
}

// OnChange registers fn, run (on its own goroutine) whenever the list of
// servers or their addresses may have changed.
func (s *Servers) OnChange(fn func()) {
	s.mu.Lock()
	s.watch = append(s.watch, fn)
	s.mu.Unlock()
}

func (s *Servers) changed() {
	s.mu.Lock()
	fns := slices.Clone(s.watch)
	s.mu.Unlock()
	for _, fn := range fns {
		go fn()
	}
}

// Load folds every node.addr row in st.
func (s *Servers) Load(ctx context.Context, st *store.Store) error {
	rows := map[string]serverRow{}
	f := store.Filter{Kinds: []string{store.KindNodeAddr}, Limit: store.MaxListLimit}
	for {
		evs, err := st.List(ctx, f)
		if err != nil {
			return err
		}
		for _, ev := range evs {
			foldAddr(rows, ev)
		}
		if len(evs) < f.Limit {
			break
		}
		f.After = evs[len(evs)-1].Cursor()
	}
	s.mu.Lock()
	s.rows = rows
	s.mu.Unlock()
	s.changed()
	return nil
}

// Follow keeps the fold current until ctx ends: node.addr rows as they are
// appended or inserted, and a reload after a resync.
func (s *Servers) Follow(ctx context.Context, st *store.Store) {
	sub := st.Subscribe(ctx)
	for ev := range sub {
		switch {
		case ev.Seq == 0:
			_ = s.Load(ctx, st)
		case ev.Kind == store.KindNodeAddr:
			s.mu.Lock()
			foldAddr(s.rows, ev)
			s.mu.Unlock()
			s.changed()
		case store.IsPersonKind(ev.Kind):
			s.changed() // a revoke takes a server out of the list
		}
	}
}

// foldAddr keeps each writer's newest row by seq.
func foldAddr(rows map[string]serverRow, ev store.Event) {
	var a NodeAddr
	if json.Unmarshal(ev.Payload, &a) != nil {
		return
	}
	if cur, ok := rows[ev.DeviceID]; ok && cur.seq > ev.Seq {
		return
	}
	rows[ev.DeviceID] = serverRow{seq: ev.Seq, addr: a}
}

// Row is the newest node.addr dev wrote, ignoring a revoked writer's.
func (s *Servers) Row(dev string) (NodeAddr, bool) {
	if dev != s.self && s.revoked(dev) {
		return NodeAddr{}, false
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	r, ok := s.rows[dev]
	return r.addr, ok
}

func (s *Servers) revoked(dev string) bool {
	if s.chain == nil {
		return false
	}
	_, revoked := s.chain.Cut(dev)
	return revoked
}

// List is every other device that runs a home server, with the addresses
// to dial it at: the LAN address its datagrams came from, its row's
// addresses (LAN first), the last one that worked, and pairing hints until
// its row arrives. A server whose row says off, and a revoked one, are left
// out.
func (s *Servers) List() []Server {
	s.mu.Lock()
	ids := map[string]bool{}
	for id := range s.rows {
		ids[id] = true
	}
	for id, c := range s.cache {
		if len(c.Hint) > 0 {
			ids[id] = true
		}
	}
	s.mu.Unlock()
	var out []Server
	for id := range ids {
		if id == s.self || s.revoked(id) {
			continue
		}
		if addrs := s.Addrs(id); len(addrs) > 0 {
			out = append(out, Server{ID: id, Addrs: addrs})
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

// Addrs is where to dial server id now (List's order, de-duplicated).
func (s *Servers) Addrs(id string) []string {
	s.mu.Lock()
	defer s.mu.Unlock()
	r, hasRow := s.rows[id]
	if hasRow && r.addr.Reach == ReachOff {
		return nil
	}
	var out []string
	add := func(a string) {
		if a != "" && !slices.Contains(out, a) {
			out = append(out, a)
		}
	}
	c := s.cache[id]
	port := r.addr.Port
	if port == 0 {
		// No row yet: a hint's port is the server's port N.
		for _, h := range c.Hint {
			if _, p, err := net.SplitHostPort(h); err == nil {
				port, _ = strconv.Atoi(p)
				break
			}
		}
	}
	if ip, ok := s.heard[id]; ok && port > 0 {
		add(net.JoinHostPort(ip.String(), strconv.Itoa(port)))
	}
	if hasRow {
		for _, a := range r.addr.Dial() {
			add(a)
		}
	}
	add(c.Last)
	if !hasRow {
		for _, h := range c.Hint {
			add(h)
		}
	}
	return out
}

// Heard records the LAN address a server's authentic datagram came from
// (the LAN carrier's OnHeard). Only a device that runs a server (has a row
// or a hint) is kept.
func (s *Servers) Heard(id string, ip netip.Addr) {
	ip = ip.Unmap()
	s.mu.Lock()
	_, row := s.rows[id]
	_, hint := s.cache[id]
	if id == s.self || (!row && !hint) || s.heard[id] == ip {
		s.mu.Unlock()
		return
	}
	s.heard[id] = ip
	s.mu.Unlock()
	s.changed()
}

// Worked records the address a hub handshake to server id completed on.
func (s *Servers) Worked(id, addr string) {
	s.mu.Lock()
	c := s.cache[id]
	if id == "" || c.Last == addr {
		s.mu.Unlock()
		return
	}
	c.Last = addr
	s.cache[id] = c
	s.mu.Unlock()
	s.save()
}

// Hint records addresses for server id that a pairing handed this device
// (§3 step 6), until its node.addr row arrives by sync.
func (s *Servers) Hint(id string, addrs []string) {
	if id == "" || id == s.self || len(addrs) == 0 {
		return
	}
	s.mu.Lock()
	c := s.cache[id]
	c.Hint = slices.Clone(addrs)
	s.cache[id] = c
	s.mu.Unlock()
	s.save()
	s.changed()
}

// Hints is every server this device can hand a device it pairs (§3 step 6):
// each one's addresses as List has them, and this device's own when it is
// a server (self's row).
func (s *Servers) Hints() map[string][]string {
	out := map[string][]string{}
	for _, sv := range s.List() {
		out[sv.ID] = sv.Addrs
	}
	if r, ok := s.Row(s.self); ok {
		if d := r.Dial(); len(d) > 0 {
			out[s.self] = d
		}
	}
	return out
}

func (s *Servers) save() {
	if s.dir == "" {
		return
	}
	s.mu.Lock()
	raw, _ := json.MarshalIndent(s.cache, "", "  ")
	s.mu.Unlock()
	path := filepath.Join(s.dir, ServersFile)
	tmp := path + ".tmp"
	if os.WriteFile(tmp, raw, 0o600) == nil {
		_ = os.Rename(tmp, path)
	}
}

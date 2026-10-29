// Package devcolor gives each of the person's devices a colour (card #22):
// its agents' faces and its single-device cards wear it. A device gets a
// default nobody else has; the person can pick another in Devices, and the
// pick replicates (a device.color row), so every device agrees.
package devcolor

import (
	"encoding/json"
	"hash/fnv"
	"sort"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/store"
)

// Kind is the replicated row a pick writes: {deviceId, color, at}. The
// newest by (hlc, device, seq) wins; "" goes back to the default.
const Kind = "device.color"

// Palette is the colours a device can wear, by name; the window maps each
// name to a CSS variable (--device-<name>). They stay clear of the status
// colours (blue working, green done, amber waiting, red blocked) and the
// person's violet. Append only: a default is its device id's hash over
// this list.
var Palette = []string{"teal", "orange", "pink", "lime", "cyan", "indigo", "sand"}

// Valid reports whether c is a palette colour.
func Valid(c string) bool {
	for _, p := range Palette {
		if p == c {
			return true
		}
	}
	return false
}

// Device is one device to colour: its id and when it joined the person.
type Device struct {
	ID string
	At time.Time
}

// Assign colours devices: a device the person picked a colour for wears
// it; the rest, in the order they joined (then by id), take the colour
// their id hashes to, or the next one along that no device holds yet, so
// defaults are unique while the palette lasts and adding a device never
// moves an older one.
func Assign(devices []Device, chosen map[string]string) map[string]string {
	ds := append([]Device{}, devices...)
	sort.Slice(ds, func(i, j int) bool {
		if !ds[i].At.Equal(ds[j].At) {
			return ds[i].At.Before(ds[j].At)
		}
		return ds[i].ID < ds[j].ID
	})
	out := map[string]string{}
	used := map[string]bool{}
	for _, d := range ds {
		if c := chosen[d.ID]; Valid(c) {
			out[d.ID] = c
			used[c] = true
		}
	}
	for _, d := range ds {
		if _, ok := out[d.ID]; ok {
			continue
		}
		h := fnv.New32a()
		_, _ = h.Write([]byte(d.ID))
		start := int(h.Sum32() % uint32(len(Palette)))
		c := Palette[start]
		for i := range Palette {
			if p := Palette[(start+i)%len(Palette)]; !used[p] {
				c = p
				break
			}
		}
		out[d.ID] = c
		used[c] = true
	}
	return out
}

type pick struct {
	color  string
	hlc    uint64
	device string
	seq    int64
}

type row struct {
	DeviceID string `json:"deviceId"`
	Color    string `json:"color"`
}

// Picks folds device.color rows: the person's pick per device.
type Picks struct {
	mu sync.Mutex
	m  map[string]pick
}

// NewPicks is an empty fold.
func NewPicks() *Picks { return &Picks{m: map[string]pick{}} }

// Apply folds one row; it reports whether the pick changed.
func (p *Picks) Apply(ev store.Event) bool {
	// Only the person picks, from the app (a user row, v0.4 §2).
	if ev.Kind != Kind || ev.Actor != "user" {
		return false
	}
	var r row
	if json.Unmarshal(ev.Payload, &r) != nil || r.DeviceID == "" || (r.Color != "" && !Valid(r.Color)) {
		return false
	}
	n := pick{color: r.Color, hlc: uint64(ev.HLC), device: ev.DeviceID, seq: ev.Seq}
	p.mu.Lock()
	defer p.mu.Unlock()
	if cur, ok := p.m[r.DeviceID]; ok && !newer(n, cur) {
		return false
	}
	p.m[r.DeviceID] = n
	return true
}

func newer(a, b pick) bool {
	if a.hlc != b.hlc {
		return a.hlc > b.hlc
	}
	if a.device != b.device {
		return a.device > b.device
	}
	return a.seq > b.seq
}

// Chosen is the person's pick per device ("" picks are left out).
func (p *Picks) Chosen() map[string]string {
	p.mu.Lock()
	defer p.mu.Unlock()
	out := map[string]string{}
	for id, k := range p.m {
		if k.color != "" {
			out[id] = k.color
		}
	}
	return out
}

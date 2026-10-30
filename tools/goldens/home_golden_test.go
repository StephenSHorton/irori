// Golden exporter for crates/home (mesh.md §2.5, §2.6, §3.3).
//
// This file is not part of the Go repo. tools/goldens/export.sh injects it
// into I:internal/home with `go test -overlay` (package home, so it reaches
// splitPublic, saveSettings and the test helpers in home_test.go). It only
// runs when IRORI_GOLDEN_OUT is set, and writes $IRORI_GOLDEN_OUT/home/home.json:
//
//	nodeAddr   json.Marshal of node.addr payloads (the off row, unmapped,
//	           mapped, manual, cgnat, IPv6, an HTML-escaped host), with
//	           Dial and Outside of each, and Same over pairs differing in at
//	           and in content
//	settings   home-server.json bytes (MarshalIndent through saveSettings)
//	           for tri-state outside/awake, external ports and a public host
//	status     json.Marshal of home.Status values, and the Status a Go
//	           server reports with a fake router for: mapped, CGNAT,
//	           mapping failed, home only, manual address (ports masked)
//	hostPort   roomnet.HostPort and splitPublic over typed addresses
//	cgnat      portmap.CGNAT over addresses
//
// Every address here is a documentation range or loopback.
package home

import (
	"encoding/json"
	"errors"
	"net/netip"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	"github.com/StephenSHorton/go-room/portmap"

	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/mesh/roomnet"
)

type goldAddr struct {
	JSON    string   `json:"json"`
	Dial    []string `json:"dial"`
	Outside string   `json:"outside"`
}

type goldSame struct {
	A    string `json:"a"`
	B    string `json:"b"`
	Same bool   `json:"same"`
}

type goldHostPort struct {
	In        string `json:"in"`
	Port      int    `json:"port"`
	Out       string `json:"out"`
	Err       string `json:"err"`
	PubHost   string `json:"pubHost"`
	PubPort   int    `json:"pubPort"`
	PubErr    string `json:"pubErr"`
}

type goldScenario struct {
	Name   string `json:"name"`
	Status string `json:"status"`
	Row    string `json:"row"`
}

func TestGoldenHome(t *testing.T) {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	out := map[string]any{}

	pub := func(ip string, tcp, udp int) *mesh.PublicAddr { return &mesh.PublicAddr{IP: ip, TCP: tcp, UDP: udp} }
	addrs := []mesh.NodeAddr{
		{V: 1, Reach: mesh.ReachOff, At: 1761700000000},
		{V: 1, Port: 25817, LAN: []string{"192.168.1.20"}, Reach: mesh.ReachUnmapped, At: 1761700000001},
		{V: 1, Port: 25817, LAN: []string{"192.168.1.20", "172.16.0.5"}, Public: pub("203.0.113.7", 25817, 25818), Via: "upnp", Reach: mesh.ReachOK, At: 2},
		{V: 1, Port: 25817, LAN: []string{"192.168.1.20"}, Public: pub("pc.example", 443, 443), Via: "manual", Reach: mesh.ReachOK, At: 3},
		{V: 1, Port: 25817, Public: pub("100.64.3.4", 25817, 25817), Via: "natpmp", Reach: mesh.ReachCGNAT, At: 4},
		{V: 1, Port: 25817, LAN: []string{"192.168.1.20"}, V6: []string{"2001:db8::7"}, Public: pub("2001:db8::1", 25817, 25817), Via: "upnp", Reach: mesh.ReachOK, At: 5},
		{V: 1, Port: 25817, Public: pub("a<b>&c.example", 1, 1), Via: "manual", Reach: mesh.ReachOK, At: 6},
		{V: 1, Port: 0, LAN: []string{"192.168.1.20"}, Reach: mesh.ReachUnmapped, At: 7},
		{V: 1, Port: 25817, Public: pub("", 25817, 0), Via: "upnp", Reach: mesh.ReachOK},
	}
	var ga []goldAddr
	for _, a := range addrs {
		raw, _ := json.Marshal(a)
		d := a.Dial()
		if d == nil {
			d = []string{}
		}
		ga = append(ga, goldAddr{JSON: string(raw), Dial: d, Outside: a.Outside()})
	}
	out["nodeAddr"] = ga
	var gs []goldSame
	for i, a := range addrs {
		b := a
		b.At += 12345
		ra, _ := json.Marshal(a)
		rb, _ := json.Marshal(b)
		gs = append(gs, goldSame{A: string(ra), B: string(rb), Same: a.Same(b)})
		c := addrs[(i+1)%len(addrs)]
		rc, _ := json.Marshal(c)
		gs = append(gs, goldSame{A: string(ra), B: string(rc), Same: a.Same(c)})
	}
	out["same"] = gs

	tru, fal := true, false
	sets := []Settings{
		{},
		{On: true, Port: 25817},
		{On: true, Port: 25817, Outside: &fal},
		{On: true, Port: 25817, Outside: &tru, Awake: &fal},
		{On: false, Port: 20001, Public: "pc.example:443", TCPExternal: 25000, UDPExternal: 25001, Awake: &tru},
		{On: true, Public: "<x>&y"},
	}
	var gset []string
	for _, s := range sets {
		d := t.TempDir()
		if err := saveSettings(d, s); err != nil {
			t.Fatal(err)
		}
		raw, err := os.ReadFile(filepath.Join(d, File))
		if err != nil {
			t.Fatal(err)
		}
		gset = append(gset, string(raw))
	}
	out["settings"] = gset

	stats := []Status{
		{},
		{On: true, Outside: true, Awake: true},
		{On: true, Outside: true, Public: "pc.example:443", Running: true, Port: 25817, Reach: ReachOutside,
			Address: "pc.example:443", Line: "a <line> & more", Awake: true, AwakeLine: "Keeping this PC awake"},
		{On: true, Running: true, Port: 25817, Reach: ReachHome, AwakeLine: "Can't keep this PC awake: x"},
	}
	var gst []string
	for _, s := range stats {
		raw, _ := json.Marshal(s)
		gst = append(gst, string(raw))
	}
	out["status"] = gst

	var ghp []goldHostPort
	for _, c := range []struct {
		in   string
		port int
	}{
		{"1.2.3.4", 25817}, {" pc.example:443 ", 1}, {"2001:db8::7", 9}, {"[2001:db8::7]:9", 1}, {"[::1]", 9},
		{"", 9}, {"   ", 9}, {"h:0", 9}, {":80", 9}, {"h:65536", 9}, {"h:+80", 9}, {"h:x", 9}, {"h", 0},
		{"not an address/", 1}, {"a b", 1}, {"pc.example", 25817}, {"[a]b:1", 1}, {"a:b:c", 0}, {"[::1]:", 1},
		{"\tpc.example:80\n", 1},
	} {
		g := goldHostPort{In: c.in, Port: c.port}
		if hp, err := roomnet.HostPort(c.in, c.port); err != nil {
			g.Err = err.Error()
		} else {
			g.Out = hp
		}
		if h, p, err := splitPublic(c.in, c.port); err != nil {
			g.PubErr = err.Error()
		} else {
			g.PubHost, g.PubPort = h, p
		}
		ghp = append(ghp, g)
	}
	out["hostPort"] = ghp

	cg := map[string]bool{}
	for _, a := range []string{"100.64.0.1", "100.127.255.254", "100.128.0.1", "192.168.0.1", "10.1.1.1", "172.16.0.1",
		"172.32.0.1", "127.0.0.1", "203.0.113.9", "8.8.8.8", "fd00::1", "2001:db8::1", "::1"} {
		cg[a] = portmap.CGNAT(netip.MustParseAddr(a))
	}
	out["cgnat"] = cg

	// A Go server with a fake router, as home_test.go runs it: Status and
	// the row it wrote, with the random port masked as N.
	var scen []goldScenario
	run := func(name string, setup func(n *node), on, outside bool, public string, done func(n *node) bool) {
		n := newNode(t, nil)
		if setup != nil {
			setup(n)
		}
		n.run()
		if err := n.srv.Set(on, outside, public); err != nil {
			t.Fatal(err)
		}
		eventually(t, name, func() bool { return done(n) })
		st, _ := json.Marshal(n.srv.Status())
		rows := n.rows()
		row := rows[len(rows)-1]
		row.At = 0
		rr, _ := json.Marshal(row)
		port := strconv.Itoa(n.srv.Settings().Port)
		mask := func(s string) string { return strings.ReplaceAll(s, port, "N") }
		scen = append(scen, goldScenario{Name: name, Status: mask(string(st)), Row: mask(string(rr))})
		n.cancel()
	}
	one := func(reach string) func(n *node) bool {
		return func(n *node) bool { r := n.rows(); return len(r) == 1 && r[0].Reach == reach }
	}
	run("mapped", nil, true, true, "", one(mesh.ReachOK))
	run("cgnat", func(n *node) { n.gw.ext = netip.MustParseAddr("100.64.3.4") }, true, true, "", one(mesh.ReachCGNAT))
	run("failed", func(n *node) { n.gw.fail = errors.New("no IGD") }, true, true, "", one(mesh.ReachUnmapped))
	run("home", nil, true, false, "", one(mesh.ReachUnmapped))
	run("manual", nil, true, true, "pc.example:443", one(mesh.ReachOK))
	out["scenarios"] = scen

	raw, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(dir, "home"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "home", "home.json"), append(raw, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}

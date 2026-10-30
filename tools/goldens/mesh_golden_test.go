// Golden exporter for crates/mesh (mesh.md §2.1, §2.4, §2.5, §2.6; D12).
//
// This file is not part of the Go repo. tools/goldens/export.sh injects it
// into I:internal/mesh with `go test -overlay` (package mesh, so it reaches
// secretKey and the unexported file types). It only runs when
// IRORI_GOLDEN_OUT is set, and writes $IRORI_GOLDEN_OUT/mesh/mesh.json:
//
//	meshSecret  MeshSecret(seed, personId, epoch): key and topic
//	sealed      person.secret sealing with a fixed nonce, and openSecret
//	frames      Go-encoded sync frames, every field the Rust side sets
//	nodeAddr    node.addr payloads, Same and Dial
//	files       mesh-peers.json, hub-addrs.json and relay.json bytes
//	status      PeerState, Status and DeviceRow JSON (durations, times)
//	scrub       ScrubURLs
package mesh

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"

	"golang.org/x/crypto/nacl/secretbox"

	"github.com/StephenSHorton/irori/internal/store"
)

type goldSecret struct {
	Seed     string `json:"seed"`
	PersonID string `json:"personId"`
	Epoch    int    `json:"epoch"`
	Key      string `json:"key"`
	Topic    string `json:"topic"`
}

type goldSealed struct {
	Old       string `json:"old"`
	New       string `json:"new"`
	PersonID  string `json:"personId"`
	Epoch     int    `json:"epoch"`
	Recipient string `json:"recipient"`
	Nonce     string `json:"nonce"`
	Sealed    string `json:"sealed"`
	Opens     bool   `json:"opens"`
}

type goldFrame struct {
	Name string          `json:"name"`
	JSON json.RawMessage `json:"json"`
	Text string          `json:"text"`
}

type goldAddr struct {
	Name    string   `json:"name"`
	Payload string   `json:"payload"`
	Dial    []string `json:"dial"`
	Outside string   `json:"outside"`
	SameAt  bool     `json:"sameAt"`
}

type goldText struct {
	Name string `json:"name"`
	Text string `json:"text"`
}

func goldSeed(b byte) []byte {
	s := make([]byte, 32)
	for i := range s {
		s[i] = b + byte(i)
	}
	return s
}

func TestGoldenMesh(t *testing.T) {
	out := os.Getenv("IRORI_GOLDEN_OUT")
	if out == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	var g struct {
		MeshSecret []goldSecret `json:"meshSecret"`
		Sealed     []goldSealed `json:"sealed"`
		Frames     []goldFrame  `json:"frames"`
		NodeAddr   []goldAddr   `json:"nodeAddr"`
		Files      []goldText   `json:"files"`
		Status     []goldText   `json:"status"`
		Scrub      []goldText   `json:"scrub"`
	}

	for _, c := range []struct {
		seed  []byte
		pid   string
		epoch int
	}{
		{goldSeed(0), "p", 1}, {goldSeed(0), "p", 2}, {goldSeed(7), "0123456789abcdef0123456789abcdef", 1},
		{goldSeed(9), "person-ü", 3}, {make([]byte, 32), "", 0}, {goldSeed(1), "p", 1 << 40},
	} {
		s := MeshSecret(c.seed, c.pid, c.epoch)
		g.MeshSecret = append(g.MeshSecret, goldSecret{hex.EncodeToString(c.seed), c.pid, c.epoch,
			hex.EncodeToString(s.Key[:]), s.Topic})
	}

	for i, c := range []struct {
		pid, rcpt string
		epoch     int
	}{{"p", "dev-b", 2}, {"0123456789abcdef", "0123456789abcdef0123456789abcdef", 5}, {"p", "", 1}} {
		old, nw := goldSeed(byte(10*i)), goldSeed(byte(100+i))
		var nonce [24]byte
		for j := range nonce {
			nonce[j] = byte(i*24 + j)
		}
		k, err := secretKey(old, c.pid, c.epoch, c.rcpt)
		if err != nil {
			t.Fatal(err)
		}
		sealed := secretbox.Seal(nonce[:], nw, &nonce, k)
		got, err := openSecret(old, sealed, c.pid, c.epoch, c.rcpt)
		g.Sealed = append(g.Sealed, goldSealed{hex.EncodeToString(old), hex.EncodeToString(nw), c.pid, c.epoch, c.rcpt,
			hex.EncodeToString(nonce[:]), hex.EncodeToString(sealed), err == nil && string(got) == string(nw)})
	}

	row := Row{D: "dev-a", S: 7, H: 1<<62 + 5, A: "agent:dev-a/s<1>&", K: "agent.seen", P: []byte(`{"b": 2 , "t":"<&>"}`), Sig: []byte{1, 2, 3, 250}}
	frames := []struct {
		name string
		f    Frame
	}{
		{"hello", Frame{Type: FrameHello, V: Protocol, DeviceID: "dev-a", PersonID: "p1", Epoch: 2, CoreVersion: "0.5.0",
			Schema: 3, WallMs: 1761700000123, Vector: store.Vector{"dev-b": 12, "dev-a": 40, "dev-c": 0},
			Top: "ab12", TopRow: &row, PersonPub: "ff00"}},
		{"helloEmpty", Frame{Type: FrameHello, V: Protocol, DeviceID: "dev-a", PersonID: "p1", Epoch: 1, Schema: 3, Vector: store.Vector{}}},
		{"rows", Frame{Type: FrameRows, Events: []Row{row, {D: "dev-b", S: 1, H: 9, A: "irori", K: "x"}}, Batch: 3}},
		{"rowsLoose", Frame{Type: FrameRows, Events: []Row{row}, Loose: true}},
		{"vectorAck", Frame{Type: FrameVector, Vector: store.Vector{"d": 1 << 50}, Ack: true, Batch: 9}},
		{"vector", Frame{Type: FrameVector, Vector: store.Vector{"d": 3}}},
		{"ping", Frame{Type: FramePing, T: 1761700000999}},
		{"pong", Frame{Type: FramePong, T: 1761700000999}},
		{"personSecret", Frame{Type: FramePersonSecret, Epoch: 3, From: 1, Sealed: []byte("0123456789abcdef0123456789abcdef0123456789")}},
		{"rowAt", Frame{Type: FrameRowAt, DeviceID: "dev-a", Seq: 41}},
		{"bare", Frame{Type: "future.kind"}},
	}
	for _, c := range frames {
		b, err := c.f.Encode()
		if err != nil {
			t.Fatal(err)
		}
		g.Frames = append(g.Frames, goldFrame{c.name, b, string(b)})
	}
	// The board stream bodies.
	req, _ := json.Marshal(boardRequest{Vector: store.Vector{"dev-a": 3}})
	g.Frames = append(g.Frames, goldFrame{"boardRequest", req, string(req)})
	chunk, _ := json.Marshal(boardChunk{Events: []Row{row}})
	g.Frames = append(g.Frames, goldFrame{"boardChunk", chunk, string(chunk)})
	breq, _ := json.Marshal(blobRequest{ID: "0011223344556677", SHA256: "ab", Offset: 5, Len: 0})
	g.Frames = append(g.Frames, goldFrame{"blobRequest", breq, string(breq)})
	offer, _ := json.Marshal(FileOffer{SHA256: "ab", Size: 12, Mime: "text/plain"})
	g.Frames = append(g.Frames, goldFrame{"fileOffer", offer, string(offer)})
	pres, _ := json.Marshal(Presence{DeviceID: "dev-a", Epoch: 2, CoreVersion: "0.5.0"})
	g.Frames = append(g.Frames, goldFrame{"presence", pres, string(pres)})

	for _, c := range []struct {
		name string
		a    NodeAddr
	}{
		{"pc", NodeAddr{V: 1, Port: 25817, LAN: []string{"192.168.1.20"}, Public: nil, Via: "", Reach: ReachUnmapped, At: 1761700000000}},
		{"mapped", NodeAddr{V: 1, Port: 24817, LAN: []string{"192.168.1.20", "10.0.0.2"}, V6: []string{"2001:db8::1"},
			Public: &PublicAddr{IP: "203.0.113.7", TCP: 24900, UDP: 24901}, Via: "upnp", Reach: ReachOK, At: 5}},
		{"cgnat", NodeAddr{V: 1, Port: 24817, Public: &PublicAddr{IP: "100.64.0.1", TCP: 24817, UDP: 24817}, Via: "natpmp", Reach: ReachCGNAT, At: 6}},
		{"off", NodeAddr{V: 1, Public: nil, Via: "", Reach: ReachOff, At: 7}},
	} {
		b, _ := json.Marshal(c.a)
		moved := c.a
		moved.At++
		g.NodeAddr = append(g.NodeAddr, goldAddr{c.name, string(b), c.a.Dial(), c.a.Outside(), c.a.Same(moved)})
	}

	peers, _ := json.MarshalIndent(map[string]peerEpoch{"dev-b": {Epoch: 2, PersonPub: "ff00"}, "dev-a": {Epoch: 1}}, "", "  ")
	g.Files = append(g.Files, goldText{"mesh-peers.json", string(peers)})
	cache, _ := json.MarshalIndent(map[string]serverCache{"dev-z": {Last: "192.168.1.2:25817"}, "dev-y": {Hint: []string{"10.0.0.2:1", "[::1]:2"}}}, "", "  ")
	g.Files = append(g.Files, goldText{"hub-addrs.json", string(cache)})
	relay, _ := json.MarshalIndent(Relay{CredentialURL: "https://relay.example/cred?k=1", TurnURLs: []string{"turns:relay.example:5349"}}, "", "  ")
	g.Files = append(g.Files, goldText{"relay.json", string(relay)})

	at := time.Date(2026, 10, 29, 12, 30, 45, 123456700, time.UTC)
	ps := PeerState{DeviceID: "dev-b", Label: "Mac", Role: "desktop", State: "connected", Epoch: 2, Path: "direct",
		RTT: 1500 * time.Microsecond, LastPing: at, LastSync: at.Add(-time.Second), Skew: -250 * time.Millisecond,
		Refused: 1, Ignored: 2, Held: 3, Stopped: "its clock went backwards", Ahead: true, Old: true}
	b, _ := json.Marshal(ps)
	g.Status = append(g.Status, goldText{"peerFull", string(b)})
	b, _ = json.Marshal(PeerState{DeviceID: "dev-c", Label: "", Role: "node", State: "offline", Revoked: true, Pending: 1, Remaining: 2})
	g.Status = append(g.Status, goldText{"peerRevoked", string(b)})
	b, _ = json.Marshal(Status{State: StateOn, Epoch: 2, Topics: 1, Peers: []PeerState{ps}})
	g.Status = append(g.Status, goldText{"status", string(b)})
	b, _ = json.Marshal(Status{State: StateOff, Epoch: 1})
	g.Status = append(g.Status, goldText{"statusEmpty", string(b)})
	b, _ = json.Marshal(DeviceRow{DeviceID: "dev-a", Name: "PC", Role: "desktop", OS: "windows", This: true, State: "this"})
	g.Status = append(g.Status, goldText{"rowThis", string(b)})
	d := ps
	b, _ = json.Marshal(DeviceRow{DeviceID: "dev-b", Name: "Mac", Role: "desktop", State: "connected", Warning: "It its clock went backwards. Remove it.",
		Action: "remove", LastSeen: at, Detail: &d})
	g.Status = append(g.Status, goldText{"rowDetail", string(b)})

	for _, s := range []string{
		"fetch https://relay.example/cred?key=abc failed",
		"two http://a/b#frag and https://c/d?x=1&y=2\tend",
		`quoted "https://x/y?z=1" and 'http://q/?r'`,
		"no url here",
	} {
		g.Scrub = append(g.Scrub, goldText{s, ScrubURLs(s)})
	}

	raw, err := json.MarshalIndent(g, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	dir := filepath.Join(out, "mesh")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "mesh.json"), append(raw, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}

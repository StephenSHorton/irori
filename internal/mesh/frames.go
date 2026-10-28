package mesh

import (
	"encoding/json"
	"fmt"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

// Protocol is the sync protocol version a hello carries (§5.2).
const Protocol = 1

// Frame types on the sync channel (§5.2). Each frame is one Conn message.
const (
	FrameHello        = "hello"
	FrameRows         = "rows"
	FrameVector       = "vector"
	FramePing         = "ping"
	FramePong         = "pong"
	FramePersonSecret = "person.secret"
	// FrameRowAt asks the peer for its copy of the asker's own row at Seq
	// (DeviceID is the asker): a restored device checks the last row it
	// holds against the peer's before it takes the rows after it (§5.2).
	FrameRowAt = "row.at"
)

// Limits on a rows frame (§5.2, §8): Go sends at most 500 rows and about
// 1 MiB per frame; MaxMessage is the channel's cap on anything it receives.
const (
	MaxBatchRows  = 500
	MaxBatchBytes = 1 << 20
	MaxMessage    = 8 << 20
)

// Frame is one sync message. Type picks which fields are set.
type Frame struct {
	Type string `json:"type"`

	// hello
	V           int          `json:"v,omitempty"`
	DeviceID    string       `json:"deviceId,omitempty"`
	PersonID    string       `json:"personId,omitempty"`
	Epoch       int          `json:"epoch,omitempty"`
	CoreVersion string       `json:"coreVersion,omitempty"`
	Schema      int          `json:"schema,omitempty"`
	WallMs      int64        `json:"wallMs,omitempty"`
	Vector      store.Vector `json:"vector,omitempty"` // hello and vector
	// Ack marks the vector sent after applying a rows frame, as opposed to
	// the periodic one: the sender sends its next batch on it.
	Ack bool `json:"ack,omitempty"`
	// Top (hello) is the RowHash of the receiver's own row at
	// Vector[receiver], as the sender holds it. A device whose own row
	// there differs was restored from a copy and wrote over rows it had
	// already sent: a fork (§5.2). Vectors alone can't see it.
	Top string `json:"top,omitempty"`
	// TopRow (hello) is that row itself. When the sender holds more of the
	// receiver's rows than the receiver does, its signature, by the
	// receiver's own key, proves the receiver really wrote that far, so a
	// peer can't hold the receiver's appends with a made-up vector.
	TopRow *Row `json:"topRow,omitempty"`
	// Seq (row.at) is the seq asked for.
	Seq int64 `json:"seq,omitempty"`

	// rows. Loose rows are not in (device, seq) order: the person.* rows a
	// peer lacks and fork rows, sent first on each connection; each is
	// inserted on its own, and a person row that can't be inserted yet
	// (its device's earlier rows are missing) is learned (§5.2).
	Events []Row `json:"events,omitempty"`
	Loose  bool  `json:"loose,omitempty"`

	// ping / pong
	T int64 `json:"t,omitempty"`

	// PersonPub (hello) is the sender's person key for its Epoch. Two
	// devices that rotated to one epoch apart hold different keys, and so
	// different topics; this is how they see it (§3, Revoke).
	PersonPub string `json:"personPub,omitempty"`

	// person.secret (Epoch too): the new epoch's secret, sealed (§3) with
	// the key of epoch From (0: the recipient's current epoch).
	Sealed []byte `json:"sealed,omitempty"`
	From   int    `json:"from,omitempty"`
}

// Row is a log row on the wire. The payload travels as base64 of the
// stored bytes, so the signed bytes survive the trip (§5.2).
type Row struct {
	D   string `json:"d"`
	S   int64  `json:"s"`
	H   uint64 `json:"h"`
	A   string `json:"a"`
	K   string `json:"k"`
	P   []byte `json:"p"`   // base64 in JSON
	Sig []byte `json:"sig"` // base64 in JSON
}

// RowOf is ev on the wire.
func RowOf(ev store.Event) Row {
	return Row{D: ev.DeviceID, S: ev.Seq, H: uint64(ev.HLC), A: ev.Actor, K: ev.Kind, P: ev.Payload, Sig: ev.Sig}
}

// Event is r as a store row.
func (r Row) Event() store.Event {
	return store.Event{DeviceID: r.D, Seq: r.S, HLC: hlc.Timestamp(r.H), Actor: r.A, Kind: r.K, Payload: json.RawMessage(r.P), Sig: r.Sig}
}

// size estimates r's encoded bytes.
func (r Row) size() int {
	return 96 + len(r.D) + len(r.A) + len(r.K) + (len(r.P)+len(r.Sig))*4/3
}

// Encode marshals f.
func (f Frame) Encode() ([]byte, error) { return json.Marshal(f) }

// DecodeFrame unmarshals one sync message.
func DecodeFrame(b []byte) (Frame, error) {
	var f Frame
	if err := json.Unmarshal(b, &f); err != nil {
		return Frame{}, fmt.Errorf("mesh: bad frame: %w", err)
	}
	if f.Type == "" {
		return Frame{}, fmt.Errorf("mesh: frame has no type")
	}
	return f, nil
}

// batches cuts evs into rows frames of at most MaxBatchRows rows and about
// MaxBatchBytes each, in order. A single row larger than the byte cap goes
// alone (the channel's cap, MaxMessage, still bounds it).
func batches(evs []store.Event, loose bool) []Frame {
	var out []Frame
	var cur []Row
	size := 0
	for _, ev := range evs {
		r := RowOf(ev)
		if len(cur) > 0 && (len(cur) >= MaxBatchRows || size+r.size() > MaxBatchBytes) {
			out = append(out, Frame{Type: FrameRows, Events: cur, Loose: loose})
			cur, size = nil, 0
		}
		cur = append(cur, r)
		size += r.size()
	}
	if len(cur) > 0 {
		out = append(out, Frame{Type: FrameRows, Events: cur, Loose: loose})
	}
	return out
}

// Package mesh is v0.4's net layer (design/v0.4-devices.md §4, §5): it finds
// the person's other devices on the mesh topic, keeps one authenticated
// connection to each, and replicates the event log over it.
//
// Nothing here talks to pion or MQTT. The Core side of sync (Node) runs on a
// Transport: go-room's rooms in internal/mesh/roomnet, or FakeNet, an
// in-memory transport for tests that can partition and drop links.
package mesh

import (
	"context"
	"crypto/ed25519"
	"crypto/hkdf"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"time"
)

// Secret names one room: the key its envelopes are sealed with and the
// broker topic it meets on (go-room's room.Secret).
type Secret struct {
	Key   [32]byte
	Topic string
}

// MeshApp is the mesh topic's app name: the HKDF salt and the topic prefix.
const MeshApp = "irori/mesh/1"

// MeshSecret derives the mesh topic of one epoch from that epoch's person
// secret (§4.2):
//
//	HKDF-SHA256(ikm = person secret, salt = "irori/mesh/1", info = personId || u64(epoch))
//	→ 32-byte sealing key || 16-byte topic suffix; topic = "irori/mesh/1/" + hex(suffix)
//
// It is go-room's SecretFromKey(MeshApp, seed, info), written out here so
// this package needs no go-room import (roomnet's tests check they agree).
func MeshSecret(seed []byte, personID string, epoch int) Secret {
	dk, err := hkdf.Key(sha256.New, seed, []byte(MeshApp), string(MeshInfo(personID, epoch)), 48)
	if err != nil {
		panic(err) // 48 bytes always fits
	}
	var s Secret
	copy(s.Key[:], dk[:32])
	s.Topic = MeshApp + "/" + hex.EncodeToString(dk[32:])
	return s
}

// MeshInfo is the HKDF info of an epoch's mesh topic: personId || u64(epoch).
func MeshInfo(personID string, epoch int) []byte {
	return binary.BigEndian.AppendUint64([]byte(personID), uint64(epoch))
}

// Envelope is one signaling message from another member of a room: its
// sender, type and JSON body. Mesh's own rtc.* envelopes never arrive here.
type Envelope struct {
	From string
	Type string
	Body json.RawMessage
}

// RoomConfig sets up this device in one room.
type RoomConfig struct {
	// Self is this device's id.
	Self string
	// Pub and Sign are the device key: the handshake binds each connection
	// to it (§4.3). Sign is a function, so the key stays in identity.
	Pub  ed25519.PublicKey
	Sign func(msg []byte) []byte
	// Accept decides a peer once its handshake verified the key it holds.
	// A non-nil error refuses it before any sync frame.
	Accept func(peer string, pub ed25519.PublicKey) error
	// Allow is asked before any connection is attempted with peer.
	Allow func(peer string) bool
	// OnMsg gets every non-rtc envelope for everyone or for Self.
	OnMsg func(Envelope)
	// OnConnect fires when signaling (re)connects: announce presence.
	OnConnect func()
	// OnPeer gets each connection once it is up.
	OnPeer func(Conn)
}

// Transport opens rooms. go-room (roomnet) and FakeNet implement it.
type Transport interface {
	Join(s Secret, cfg RoomConfig) (Room, error)
}

// Room is this device's place on one topic.
type Room interface {
	// Send seals and publishes one envelope to member to ("" = everyone).
	Send(to, typ string, body any) error
	// Connect makes (or asks for) a connection to peer; idempotent. Which
	// side offers is the transport's rule (go-room: the lower id).
	Connect(peer string)
	// Close leaves the room and closes its connections.
	Close()
}

// Conn is one authenticated connection to another device.
type Conn interface {
	// ID is the peer's device id; Pub the key its handshake proved.
	ID() string
	Pub() ed25519.PublicKey
	// Send and Recv move whole messages on the one reliable, ordered
	// "sync" channel.
	Send(ctx context.Context, msg []byte) error
	Recv(ctx context.Context) ([]byte, error)
	// OpenStream opens a byte stream on its own channel (blobs, §4.3);
	// OnStream handles the ones the peer opens.
	OpenStream(ctx context.Context, label string) (io.ReadWriteCloser, error)
	OnStream(func(label string, s io.ReadWriteCloser))
	// Done is closed once the connection is down; Err says why.
	Done() <-chan struct{}
	Err() error
	Close(reason string)
	// Path is "direct" or "relay"; RTT the latest round trip.
	Path() string
	RTT() time.Duration
	// Fingerprints are the DTLS certificates' as the transport saw them,
	// for the pairing SAS (§3).
	Fingerprints() (local, remote string)
}

// ErrRefused is Accept's answer to a device that has no valid, unrevoked
// cert for this person, or proved a key its cert doesn't name.
var ErrRefused = errors.New("mesh: device is not one of this person's")

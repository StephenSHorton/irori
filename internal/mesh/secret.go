package mesh

import (
	"crypto/hkdf"
	"crypto/rand"
	"crypto/sha256"
	"encoding/binary"
	"errors"

	"golang.org/x/crypto/nacl/secretbox"
)

// The person.secret frame (§3, §5.2) carries a new epoch's person secret to
// a device still on an older epoch. The sync channel is already DTLS bound
// to the recipient's device key (§4.3); the secret is also sealed with a key
// only holders of the recipient's current person secret can derive, bound
// to the new epoch and the recipient's device id, so a frame can't be
// replayed to another device or read off a log of the channel by anyone
// who never held the person secret.
const secretApp = "irori/person-secret/1"

func secretKey(oldSeed []byte, personID string, epoch int, recipient string) (*[32]byte, error) {
	info := binary.BigEndian.AppendUint64([]byte(personID), uint64(epoch))
	info = append(info, recipient...)
	dk, err := hkdf.Key(sha256.New, oldSeed, []byte(secretApp), string(info), 32)
	if err != nil {
		return nil, err
	}
	var k [32]byte
	copy(k[:], dk)
	return &k, nil
}

// sealSecret seals newSeed (epoch's secret) for recipient, who holds oldSeed.
func sealSecret(oldSeed, newSeed []byte, personID string, epoch int, recipient string) ([]byte, error) {
	k, err := secretKey(oldSeed, personID, epoch, recipient)
	if err != nil {
		return nil, err
	}
	var nonce [24]byte
	if _, err := rand.Read(nonce[:]); err != nil {
		return nil, err
	}
	return secretbox.Seal(nonce[:], newSeed, &nonce, k), nil
}

var errBadSecret = errors.New("mesh: person.secret does not open")

// openSecret opens a sealSecret result on the recipient (self).
func openSecret(oldSeed, sealed []byte, personID string, epoch int, self string) ([]byte, error) {
	if len(sealed) < 24+secretbox.Overhead {
		return nil, errBadSecret
	}
	k, err := secretKey(oldSeed, personID, epoch, self)
	if err != nil {
		return nil, err
	}
	var nonce [24]byte
	copy(nonce[:], sealed[:24])
	out, ok := secretbox.Open(nil, sealed[24:], &nonce, k)
	if !ok {
		return nil, errBadSecret
	}
	return out, nil
}

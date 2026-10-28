package identity

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"path/filepath"
	"time"
)

// Current is the person with its current epoch and key, which a rotation
// moves (§3, Revoke).
func (id *Identity) Current() Person {
	id.mu.Lock()
	defer id.mu.Unlock()
	return id.Person
}

// PersonSeed returns the person secret (the ed25519 seed) of epoch: the
// current one, or the one before it while it is kept (§4.2). The mesh
// derives the topic of that epoch from it and seals a person.secret frame
// with it; it never goes into a row.
func (id *Identity) PersonSeed(epoch int) ([]byte, bool) {
	id.mu.Lock()
	defer id.mu.Unlock()
	switch {
	case epoch == id.Person.Epoch:
		return id.personKey.Seed(), true
	case id.prev != nil && epoch == id.prev.epoch:
		return id.prev.key.Seed(), true
	}
	return nil, false
}

// PrevEpoch is the epoch kept from before the last rotation (0 when none).
func (id *Identity) PrevEpoch() int {
	id.mu.Lock()
	defer id.mu.Unlock()
	if id.prev == nil {
		return 0
	}
	return id.prev.epoch
}

// Rotate makes a new person key (decision D8: on revoke), moves to the next
// epoch and keeps the old key as the previous epoch. The caller appends the
// returned person.key.rotated row and hands the new secret to the remaining
// devices in person.secret frames.
func (id *Identity) Rotate(at time.Time) (KeyRotated, error) {
	_, key, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		return KeyRotated{}, err
	}
	id.mu.Lock()
	defer id.mu.Unlock()
	next := id.Person.Epoch + 1
	if err := id.adoptLocked(next, key); err != nil {
		return KeyRotated{}, err
	}
	return KeyRotated{PersonID: id.Person.ID, Epoch: next, PersonPub: id.Person.Pub, At: at.UTC()}, nil
}

// ErrStaleEpoch: a person secret for an epoch this device already has or
// has passed.
var ErrStaleEpoch = errors.New("identity: person secret is not for a newer epoch")

// AdoptSecret takes the person secret of a newer epoch, received in a
// person.secret frame (§5.2). pubHex is the person.key.rotated row's key for
// that epoch; the seed must match it.
func (id *Identity) AdoptSecret(epoch int, seed []byte, pubHex string) error {
	if len(seed) != ed25519.SeedSize {
		return fmt.Errorf("identity: person secret is %d bytes", len(seed))
	}
	key := ed25519.NewKeyFromSeed(seed)
	if hex.EncodeToString(key.Public().(ed25519.PublicKey)) != pubHex {
		return errors.New("identity: person secret does not match its rotation row")
	}
	id.mu.Lock()
	defer id.mu.Unlock()
	if epoch <= id.Person.Epoch {
		return ErrStaleEpoch
	}
	return id.adoptLocked(epoch, key)
}

// adoptLocked makes key the person key of epoch, keeps the current one as
// the previous epoch and rewrites person.json. id.mu is held.
func (id *Identity) adoptLocked(epoch int, key ed25519.PrivateKey) error {
	prev := &epochKey{epoch: id.Person.Epoch, key: id.personKey}
	p := id.Person
	p.Epoch, p.Pub = epoch, hex.EncodeToString(key.Public().(ed25519.PublicKey))
	ks := id.ks
	if ks == nil {
		ks = FileKeystore{}
	}
	if err := writePerson(filepath.Join(id.dataDir, PersonFile), ks, p, key, prev); err != nil {
		return err
	}
	id.Person.Epoch, id.Person.Pub = p.Epoch, p.Pub
	id.personKey, id.prev = key, prev
	return nil
}

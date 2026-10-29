package identity

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"path/filepath"
	"time"

	"github.com/StephenSHorton/irori/internal/device"
)

// Current is the person with its current epoch and key, which a rotation
// moves (§3, Revoke).
func (id *Identity) Current() Person {
	id.mu.Lock()
	defer id.mu.Unlock()
	return id.Person
}

// Self is this device's identity, its PersonID current after a pairing
// (AdoptPerson). Device.ID, Name and OS never change after Load and may be
// read directly; read PersonID through Self or Current.
func (id *Identity) Self() device.Identity {
	id.mu.Lock()
	defer id.mu.Unlock()
	return id.Device
}

// PersonSeed returns the person secret (the ed25519 seed) of epoch: the
// current one, or an older one while it is kept (§4.2). The mesh derives
// the topic of that epoch from it and seals a person.secret frame with it;
// it never goes into a row.
func (id *Identity) PersonSeed(epoch int) ([]byte, bool) {
	id.mu.Lock()
	defer id.mu.Unlock()
	if epoch == id.Person.Epoch {
		return id.personKey.Seed(), true
	}
	for _, o := range id.older {
		if o.epoch == epoch {
			return o.key.Seed(), true
		}
	}
	return nil, false
}

// PrevEpoch is the newest epoch kept from before the current one (0 when
// none).
func (id *Identity) PrevEpoch() int {
	id.mu.Lock()
	defer id.mu.Unlock()
	if len(id.older) == 0 {
		return 0
	}
	return id.older[len(id.older)-1].epoch
}

// OlderEpochs are the epochs kept from before the current one, oldest
// first.
func (id *Identity) OlderEpochs() []int {
	id.mu.Lock()
	defer id.mu.Unlock()
	out := make([]int, 0, len(id.older))
	for _, o := range id.older {
		out = append(out, o.epoch)
	}
	return out
}

// Rotate makes a new person key (decision D8: on revoke), moves to the next
// epoch and keeps the old key as an older epoch. The caller appends the
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
	key, err := keyFor(seed, pubHex)
	if err != nil {
		return err
	}
	id.mu.Lock()
	defer id.mu.Unlock()
	if epoch <= id.Person.Epoch {
		return ErrStaleEpoch
	}
	return id.adoptLocked(epoch, key)
}

// ReplaceSecret swaps the current epoch's key for another device's key of
// the same epoch. Two devices that revoked while they couldn't reach each
// other both rotated to this epoch with different keys; the rotation row
// that wins (the mesh picks one by a total order) names pubHex, and the
// losing side takes the winner's secret so both meet on one topic again.
// The losing key is dropped.
func (id *Identity) ReplaceSecret(epoch int, seed []byte, pubHex string) error {
	key, err := keyFor(seed, pubHex)
	if err != nil {
		return err
	}
	id.mu.Lock()
	defer id.mu.Unlock()
	if epoch != id.Person.Epoch {
		return ErrStaleEpoch
	}
	if pubHex == id.Person.Pub {
		return nil
	}
	p := id.Person
	p.Pub = pubHex
	if err := writePerson(filepath.Join(id.dataDir, PersonFile), id.keystore(), p, key, id.older); err != nil {
		return err
	}
	id.Person.Pub, id.personKey = pubHex, key
	return nil
}

// Forget drops the kept keys of epochs below epoch: once every remaining
// device has said hello with the current key, nobody is left on an older
// topic, and a revoked device that still holds an older key has nothing to
// find there (§4.2).
func (id *Identity) Forget(below int) error {
	id.mu.Lock()
	defer id.mu.Unlock()
	var keep []epochKey
	for _, o := range id.older {
		if o.epoch >= below {
			keep = append(keep, o)
		}
	}
	if len(keep) == len(id.older) {
		return nil
	}
	if err := writePerson(filepath.Join(id.dataDir, PersonFile), id.keystore(), id.Person, id.personKey, keep); err != nil {
		return err
	}
	id.older = keep
	return nil
}

func keyFor(seed []byte, pubHex string) (ed25519.PrivateKey, error) {
	if len(seed) != ed25519.SeedSize {
		return nil, fmt.Errorf("identity: person secret is %d bytes", len(seed))
	}
	key := ed25519.NewKeyFromSeed(seed)
	if hex.EncodeToString(key.Public().(ed25519.PublicKey)) != pubHex {
		return nil, errors.New("identity: person secret does not match its rotation row")
	}
	return key, nil
}

func (id *Identity) keystore() Keystore {
	if id.ks == nil {
		return FileKeystore{}
	}
	return id.ks
}

// ErrNotThisKey: AdoptPerson's seed doesn't match the person key it names.
var ErrNotThisKey = errors.New("identity: person secret does not match its key")

// AdoptPerson replaces this device's person with the one pairing handed
// over (§3 step 6): person.json is rewritten with p and its secret (no
// older epochs), and device.json's personId follows. The device key stays.
// The caller re-pins the Chain to p (Chain.Repin) and reloads the store's
// verifier.
func (id *Identity) AdoptPerson(p Person, seed []byte) error {
	if len(seed) != ed25519.SeedSize {
		return fmt.Errorf("identity: person secret is %d bytes", len(seed))
	}
	if p.ID == "" || p.RootDeviceID == "" || p.Epoch < 1 {
		return errors.New("identity: incomplete person")
	}
	key := ed25519.NewKeyFromSeed(seed)
	if hex.EncodeToString(key.Public().(ed25519.PublicKey)) != p.Pub {
		return ErrNotThisKey
	}
	id.mu.Lock()
	defer id.mu.Unlock()
	if err := writePerson(filepath.Join(id.dataDir, PersonFile), id.keystore(), p, key, nil); err != nil {
		return err
	}
	dev := id.Device
	dev.PersonID = p.ID
	if err := device.Save(id.dataDir, dev); err != nil {
		return err
	}
	// Only PersonID changes: Device's ID, Name and OS are fixed after Load
	// and read without the lock (home, mesh, pair), so the struct is not
	// reassigned as a whole.
	id.Device.PersonID = dev.PersonID
	id.Person, id.personKey, id.older = p, key, nil
	return nil
}

// adoptLocked makes key the person key of epoch, keeps the current one as
// an older epoch and rewrites person.json. id.mu is held.
func (id *Identity) adoptLocked(epoch int, key ed25519.PrivateKey) error {
	older := append(append([]epochKey(nil), id.older...), epochKey{epoch: id.Person.Epoch, key: id.personKey})
	p := id.Person
	p.Epoch, p.Pub = epoch, hex.EncodeToString(key.Public().(ed25519.PublicKey))
	if err := writePerson(filepath.Join(id.dataDir, PersonFile), id.keystore(), p, key, older); err != nil {
		return err
	}
	id.Person.Epoch, id.Person.Pub = p.Epoch, p.Pub
	id.personKey, id.older = key, older
	return nil
}

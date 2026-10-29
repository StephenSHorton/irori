// Package identity is v0.4's person and device identity (design/v0.4-devices.md
// §2): the person key, the device key, device certs and revokes as signed
// rows, and the cert fold (Chain) that decides which devices belong to the
// person and which "user:<deviceId>" keys are the owner's (IsOwner).
//
// Keys never leave this package: the store gets a Signer, go-room will get
// a Sign function. Secrets are sealed at rest by a Keystore (DPAPI on
// Windows, the Keychain on macOS; decision 30, D2).
package identity

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"os/user"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/store"
)

// File names in the data dir.
const (
	PersonFile    = "person.json"
	DeviceKeyFile = "device.key"
)

// personFile is person.json: the person's public parts and its sealed secret.
type personFile struct {
	PersonID     string  `json:"personId"`
	Name         string  `json:"name"`
	Epoch        int     `json:"epoch"`
	RootDeviceID string  `json:"rootDeviceId"`
	PersonPub    string  `json:"personPub"` // hex, the current epoch's key
	SecretKey    wrapped `json:"secretKey"` // the ed25519 seed, sealed
	// Older are the epochs before the current one, oldest first, kept so a
	// device that was offline through one or more rotations still finds a
	// peer on its epoch's topic and gets the new key there (§4.2). They are
	// dropped once every remaining device has the current one (Forget).
	Older []prevKey `json:"older,omitempty"`
	// Prev is the one previous epoch an earlier build kept; read, never
	// written.
	Prev *prevKey `json:"prev,omitempty"`
}

// prevKey is the person key of an epoch before the current one.
type prevKey struct {
	Epoch     int     `json:"epoch"`
	PersonPub string  `json:"personPub"`
	SecretKey wrapped `json:"secretKey"`
}

// Person is the public view of person.json.
type Person struct {
	ID           string `json:"personId"`
	Name         string `json:"name"`
	Epoch        int    `json:"epoch"`
	RootDeviceID string `json:"rootDeviceId"`
	Pub          string `json:"personPub"`
}

// Identity is this device's keys and person. It is a store.Signer.
//
// Person is as loaded; a rotation (Rotate, AdoptSecret) moves its Epoch and
// Pub under mu, so read those through Current once the mesh runs.
type Identity struct {
	Device    device.Identity
	Person    Person
	deviceKey ed25519.PrivateKey
	dataDir   string
	ks        Keystore

	mu        sync.Mutex
	personKey ed25519.PrivateKey
	older     []epochKey // the epochs before the current one, oldest first
}

// epochKey is one epoch's person key.
type epochKey struct {
	epoch int
	key   ed25519.PrivateKey
}

var _ store.Signer = (*Identity)(nil)

// Load reads (or on first v0.4 start, creates) device.key and person.json in
// dataDir, sealing new secrets with ks, and records personId and devicePub
// in device.json. dev is device.LoadOrCreate's result.
func Load(dataDir string, dev device.Identity, ks Keystore) (*Identity, error) {
	if ks == nil {
		ks = DefaultKeystore(dataDir)
	}
	dkey, err := loadOrCreateSeed(filepath.Join(dataDir, DeviceKeyFile), ks)
	if err != nil {
		return nil, fmt.Errorf("device key: %w", err)
	}
	pubHex := hex.EncodeToString(dkey.Public().(ed25519.PublicKey))
	if dev.DevicePub != "" && dev.DevicePub != pubHex {
		return nil, fmt.Errorf("identity: device.json names key %s… but device.key holds %s…; the data dir is mixed from two installs", short(dev.DevicePub), short(pubHex))
	}
	p, pkey, older, err := loadOrCreatePerson(filepath.Join(dataDir, PersonFile), dev.ID, ks)
	if err != nil {
		return nil, fmt.Errorf("person: %w", err)
	}
	if dev.DevicePub != pubHex || dev.PersonID != p.ID {
		dev.DevicePub, dev.PersonID = pubHex, p.ID
		if err := device.Save(dataDir, dev); err != nil {
			return nil, err
		}
	}
	return &Identity{Device: dev, Person: p, deviceKey: dkey, personKey: pkey, older: older, dataDir: dataDir, ks: ks}, nil
}

// LoadOrCreateSecret reads (or on first use makes) a 32-byte random secret
// in the data dir, sealed with this device's keystore like device.key (D2).
// The home server keeps its TURN key this way (turn.key, §4.4).
func (id *Identity) LoadOrCreateSecret(name string) ([32]byte, error) {
	var out [32]byte
	k, err := loadOrCreateSeed(filepath.Join(id.dataDir, name), id.ks)
	if err != nil {
		return out, err
	}
	copy(out[:], k.Seed())
	return out, nil
}

func short(s string) string {
	if len(s) > 12 {
		return s[:12]
	}
	return s
}

func loadOrCreateSeed(path string, ks Keystore) (ed25519.PrivateKey, error) {
	raw, err := os.ReadFile(path)
	if err == nil {
		var w wrapped
		if err := json.Unmarshal(raw, &w); err != nil {
			return nil, fmt.Errorf("%s is unreadable", path)
		}
		seed, err := unseal(ks, w)
		if err != nil {
			return nil, err
		}
		if len(seed) != ed25519.SeedSize {
			return nil, fmt.Errorf("%s holds %d bytes, want an ed25519 seed", path, len(seed))
		}
		return ed25519.NewKeyFromSeed(seed), nil
	}
	if !errors.Is(err, fs.ErrNotExist) {
		return nil, err
	}
	seed := make([]byte, ed25519.SeedSize)
	if _, err := rand.Read(seed); err != nil {
		return nil, err
	}
	w, err := seal(ks, seed)
	if err != nil {
		return nil, err
	}
	if err := writeSealed(path, w); err != nil {
		return nil, err
	}
	return ed25519.NewKeyFromSeed(seed), nil
}

func loadOrCreatePerson(path, deviceID string, ks Keystore) (Person, ed25519.PrivateKey, []epochKey, error) {
	raw, err := os.ReadFile(path)
	if err == nil {
		var f personFile
		if err := json.Unmarshal(raw, &f); err != nil || f.PersonID == "" {
			return Person{}, nil, nil, fmt.Errorf("%s is unreadable", path)
		}
		key, err := unsealPersonKey(ks, f.SecretKey, f.PersonPub, path)
		if err != nil {
			return Person{}, nil, nil, err
		}
		olds := f.Older
		if f.Prev != nil && len(olds) == 0 {
			olds = []prevKey{*f.Prev}
		}
		var older []epochKey
		for _, o := range olds {
			pk, err := unsealPersonKey(ks, o.SecretKey, o.PersonPub, path)
			if err != nil {
				return Person{}, nil, nil, err
			}
			older = append(older, epochKey{epoch: o.Epoch, key: pk})
		}
		return Person{ID: f.PersonID, Name: f.Name, Epoch: f.Epoch, RootDeviceID: f.RootDeviceID, Pub: f.PersonPub}, key, older, nil
	}
	if !errors.Is(err, fs.ErrNotExist) {
		return Person{}, nil, nil, err
	}
	pub, key, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		return Person{}, nil, nil, err
	}
	p := Person{ID: PersonIDOf(pub), Name: defaultName(), Epoch: 1, RootDeviceID: deviceID, Pub: hex.EncodeToString(pub)}
	if err := writePerson(path, ks, p, key, nil); err != nil {
		return Person{}, nil, nil, err
	}
	return p, key, nil, nil
}

func unsealPersonKey(ks Keystore, w wrapped, pubHex, path string) (ed25519.PrivateKey, error) {
	seed, err := unseal(ks, w)
	if err != nil {
		return nil, err
	}
	if len(seed) != ed25519.SeedSize {
		return nil, fmt.Errorf("%s holds a bad person key", path)
	}
	key := ed25519.NewKeyFromSeed(seed)
	if hex.EncodeToString(key.Public().(ed25519.PublicKey)) != pubHex {
		return nil, fmt.Errorf("%s: personPub does not match the secret", path)
	}
	return key, nil
}

// writePerson seals key (and the older epochs') with ks and writes
// person.json atomically.
func writePerson(path string, ks Keystore, p Person, key ed25519.PrivateKey, older []epochKey) error {
	w, err := seal(ks, key.Seed())
	if err != nil {
		return err
	}
	f := personFile{
		PersonID: p.ID, Name: p.Name, Epoch: p.Epoch, RootDeviceID: p.RootDeviceID,
		PersonPub: p.Pub, SecretKey: w,
	}
	for _, o := range older {
		pw, err := seal(ks, o.key.Seed())
		if err != nil {
			return err
		}
		f.Older = append(f.Older, prevKey{Epoch: o.epoch, PersonPub: hex.EncodeToString(o.key.Public().(ed25519.PublicKey)), SecretKey: pw})
	}
	out, _ := json.MarshalIndent(f, "", "  ")
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, out, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

// defaultName is the OS user name, without a Windows domain.
func defaultName() string {
	if u, err := user.Current(); err == nil {
		name := u.Username
		if i := strings.LastIndexAny(name, `\/`); i >= 0 {
			name = name[i+1:]
		}
		if name != "" {
			return name
		}
	}
	return "you"
}

// SignRow signs a row's SigningBytes with the device key (store.Signer).
func (id *Identity) SignRow(msg []byte) ([]byte, error) {
	return ed25519.Sign(id.deviceKey, msg), nil
}

// Sign signs arbitrary bytes with the device key: the function go-room's
// handshake will get, so the key itself never leaves this package.
func (id *Identity) Sign(msg []byte) []byte { return ed25519.Sign(id.deviceKey, msg) }

// DevicePub is the device's public key.
func (id *Identity) DevicePub() ed25519.PublicKey {
	return id.deviceKey.Public().(ed25519.PublicKey)
}

func (id *Identity) signPerson(v any) (pub, sig string, err error) {
	msg, err := Canonical(v)
	if err != nil {
		return "", "", err
	}
	id.mu.Lock()
	defer id.mu.Unlock()
	return id.Person.Pub, hex.EncodeToString(ed25519.Sign(id.personKey, msg)), nil
}

// signEpoch signs canonical(v) with the current key, but only while it is
// still epoch's: v names the epoch it is signed for, and a rotation in
// between would make the two disagree.
func (id *Identity) signEpoch(epoch int, v any) (sig string, err error) {
	msg, err := Canonical(v)
	if err != nil {
		return "", err
	}
	id.mu.Lock()
	defer id.mu.Unlock()
	if id.Person.Epoch != epoch {
		return "", fmt.Errorf("identity: the person key moved to epoch %d while signing for %d", id.Person.Epoch, epoch)
	}
	return hex.EncodeToString(ed25519.Sign(id.personKey, msg)), nil
}

// CertFor makes a person.device.added payload certifying a device.
func (id *Identity) CertFor(deviceID, devicePub, label, role string, at time.Time) (DeviceAdded, error) {
	cur := id.Current()
	c := Cert{PersonID: cur.ID, DeviceID: deviceID, DevicePub: devicePub, Label: label, Role: role, At: at.UTC()}
	sig, err := id.signEpoch(cur.Epoch, c)
	return DeviceAdded{Cert: c, PersonPub: cur.Pub, PersonSig: sig, PersonEpoch: cur.Epoch}, err
}

// RevokeOf makes a person.device.revoked payload.
func (id *Identity) RevokeOf(deviceID string, cut int64, cutHash, reason string, at time.Time) (DeviceRevoked, error) {
	cur := id.Current()
	r := Revoke{PersonID: cur.ID, DeviceID: deviceID, Cut: cut, CutHash: cutHash, At: at.UTC(), Reason: reason}
	sig, err := id.signEpoch(cur.Epoch, r)
	return DeviceRevoked{Revoke: r, PersonPub: cur.Pub, PersonSig: sig, PersonEpoch: cur.Epoch}, err
}

// ProfileRow makes the person.profile payload.
func (id *Identity) ProfileRow() (Profile, error) {
	cur := id.Current()
	b := profileBody{PersonID: cur.ID, Name: cur.Name, RootDeviceID: cur.RootDeviceID, PersonPub: cur.Pub}
	_, sig, err := id.signPerson(b)
	return Profile{PersonID: b.PersonID, Name: b.Name, RootDeviceID: b.RootDeviceID, PersonPub: b.PersonPub, PersonSig: sig}, err
}

// EnsureSelfCert writes this device's first-run rows when the chain has no
// cert for it yet and it is the person's root (§2, "The root is pinned"):
// person.profile, then the self-written person.device.added. A device that
// is not the root waits for a cert from pairing (release 3) and writes
// nothing. role is RoleDesktop or RoleNode.
func (id *Identity) EnsureSelfCert(ctx context.Context, st *store.Store, ch *Chain, role string) error {
	if id.Device.ID != id.Current().RootDeviceID {
		return nil
	}
	// The cert first: it carries the key that verifies it, so it is the row
	// a peer can check before any other of this device's (Chain.Learn).
	if _, ok := ch.Cert(id.Device.ID); !ok {
		label := id.Device.Name
		if label == "" {
			label = id.Device.ID
		}
		add, err := id.CertFor(id.Device.ID, hex.EncodeToString(id.DevicePub()), label, role, time.Now())
		if err != nil {
			return err
		}
		if _, err := st.Append(ctx, "irori", store.KindPersonDeviceAdded, add); err != nil {
			return err
		}
		if _, ok := ch.Cert(id.Device.ID); !ok {
			return errors.New("identity: the self-cert row did not certify this device")
		}
	}
	if !ch.HasProfile() {
		prof, err := id.ProfileRow()
		if err != nil {
			return err
		}
		if _, err := st.Append(ctx, "irori", store.KindPersonProfile, prof); err != nil {
			return err
		}
	}
	return nil
}

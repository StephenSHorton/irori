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
type Identity struct {
	Device    device.Identity
	Person    Person
	deviceKey ed25519.PrivateKey
	personKey ed25519.PrivateKey
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
	p, pkey, err := loadOrCreatePerson(filepath.Join(dataDir, PersonFile), dev.ID, ks)
	if err != nil {
		return nil, fmt.Errorf("person: %w", err)
	}
	if dev.DevicePub != pubHex || dev.PersonID != p.ID {
		dev.DevicePub, dev.PersonID = pubHex, p.ID
		if err := device.Save(dataDir, dev); err != nil {
			return nil, err
		}
	}
	return &Identity{Device: dev, Person: p, deviceKey: dkey, personKey: pkey}, nil
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

func loadOrCreatePerson(path, deviceID string, ks Keystore) (Person, ed25519.PrivateKey, error) {
	raw, err := os.ReadFile(path)
	if err == nil {
		var f personFile
		if err := json.Unmarshal(raw, &f); err != nil || f.PersonID == "" {
			return Person{}, nil, fmt.Errorf("%s is unreadable", path)
		}
		seed, err := unseal(ks, f.SecretKey)
		if err != nil {
			return Person{}, nil, err
		}
		if len(seed) != ed25519.SeedSize {
			return Person{}, nil, fmt.Errorf("%s holds a bad person key", path)
		}
		key := ed25519.NewKeyFromSeed(seed)
		if hex.EncodeToString(key.Public().(ed25519.PublicKey)) != f.PersonPub {
			return Person{}, nil, fmt.Errorf("%s: personPub does not match the secret", path)
		}
		return Person{ID: f.PersonID, Name: f.Name, Epoch: f.Epoch, RootDeviceID: f.RootDeviceID, Pub: f.PersonPub}, key, nil
	}
	if !errors.Is(err, fs.ErrNotExist) {
		return Person{}, nil, err
	}
	pub, key, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		return Person{}, nil, err
	}
	w, err := seal(ks, key.Seed())
	if err != nil {
		return Person{}, nil, err
	}
	f := personFile{
		PersonID: PersonIDOf(pub), Name: defaultName(), Epoch: 1, RootDeviceID: deviceID,
		PersonPub: hex.EncodeToString(pub), SecretKey: w,
	}
	out, _ := json.MarshalIndent(f, "", "  ")
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, out, 0o600); err != nil {
		return Person{}, nil, err
	}
	if err := os.Rename(tmp, path); err != nil {
		return Person{}, nil, err
	}
	return Person{ID: f.PersonID, Name: f.Name, Epoch: f.Epoch, RootDeviceID: f.RootDeviceID, Pub: f.PersonPub}, key, nil
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
	return id.Person.Pub, hex.EncodeToString(ed25519.Sign(id.personKey, msg)), nil
}

// CertFor makes a person.device.added payload certifying a device.
func (id *Identity) CertFor(deviceID, devicePub, label, role string, at time.Time) (DeviceAdded, error) {
	c := Cert{PersonID: id.Person.ID, DeviceID: deviceID, DevicePub: devicePub, Label: label, Role: role, At: at.UTC()}
	pub, sig, err := id.signPerson(c)
	return DeviceAdded{Cert: c, PersonPub: pub, PersonSig: sig}, err
}

// RevokeOf makes a person.device.revoked payload.
func (id *Identity) RevokeOf(deviceID string, cut int64, cutHash, reason string, at time.Time) (DeviceRevoked, error) {
	r := Revoke{PersonID: id.Person.ID, DeviceID: deviceID, Cut: cut, CutHash: cutHash, At: at.UTC(), Reason: reason}
	pub, sig, err := id.signPerson(r)
	return DeviceRevoked{Revoke: r, PersonPub: pub, PersonSig: sig}, err
}

// ProfileRow makes the person.profile payload.
func (id *Identity) ProfileRow() (Profile, error) {
	b := profileBody{PersonID: id.Person.ID, Name: id.Person.Name, RootDeviceID: id.Person.RootDeviceID, PersonPub: id.Person.Pub}
	_, sig, err := id.signPerson(b)
	return Profile{PersonID: b.PersonID, Name: b.Name, RootDeviceID: b.RootDeviceID, PersonPub: b.PersonPub, PersonSig: sig}, err
}

// EnsureSelfCert writes this device's first-run rows when the chain has no
// cert for it yet and it is the person's root (§2, "The root is pinned"):
// person.profile, then the self-written person.device.added. A device that
// is not the root waits for a cert from pairing (release 3) and writes
// nothing. role is RoleDesktop or RoleNode.
func (id *Identity) EnsureSelfCert(ctx context.Context, st *store.Store, ch *Chain, role string) error {
	if _, ok := ch.Cert(id.Device.ID); ok {
		return nil
	}
	if id.Device.ID != id.Person.RootDeviceID {
		return nil
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
	return nil
}

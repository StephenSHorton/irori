package identity

import (
	"bytes"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"time"
)

// Roles a cert gives a device.
const (
	RoleDesktop = "desktop"
	RoleNode    = "node"
)

// Cert is what the person key signs to make a device part of the person
// (§2). It rides in a person.device.added row.
type Cert struct {
	PersonID  string    `json:"personId"`
	DeviceID  string    `json:"deviceId"`
	DevicePub string    `json:"devicePub"` // ed25519, hex
	Label     string    `json:"label"`
	Role      string    `json:"role"` // RoleDesktop or RoleNode
	At        time.Time `json:"at"`
}

// DeviceAdded is the person.device.added payload. PersonEpoch is the epoch
// of the key that signed the cert (v0.5 §3). It sits beside the cert, not
// in it: a v0.4 device re-marshals Cert from its own struct to check the
// signature, so a field it doesn't know would change the signed bytes and
// fail there. The key itself pins the epoch, so it needs no person
// signature; the row's device signature covers it.
type DeviceAdded struct {
	Cert        Cert   `json:"cert"`
	PersonPub   string `json:"personPub"` // hex
	PersonSig   string `json:"personSig"` // hex: ed25519(person key, canonical(cert))
	PersonEpoch int    `json:"personEpoch,omitempty"`
}

// Revoke is what the person key signs to remove a device.
type Revoke struct {
	PersonID string    `json:"personId"`
	DeviceID string    `json:"deviceId"`
	Cut      int64     `json:"cut"`     // the highest seq of that device the revoker held
	CutHash  string    `json:"cutHash"` // hex sha256 of that row's signed bytes and sig
	At       time.Time `json:"at"`
	Reason   string    `json:"reason,omitempty"`
}

// DeviceRevoked is the person.device.revoked payload. PersonEpoch: as
// DeviceAdded's, outside the signed Revoke for the same reason.
type DeviceRevoked struct {
	Revoke      Revoke `json:"revoke"`
	PersonPub   string `json:"personPub"`
	PersonSig   string `json:"personSig"`
	PersonEpoch int    `json:"personEpoch,omitempty"`
}

// KeyRotated is the person.key.rotated payload: the new public key, and
// since v0.5 the device that rotated (By) and PrevSig, the previous
// epoch's key over RotationBody (§3). The own mesh doesn't check By or
// PrevSig; they are kept so the rotating device can write the same
// rotation into each board as member.key.rotated, which needs both.
type KeyRotated struct {
	PersonID  string    `json:"personId"`
	Epoch     int       `json:"epoch"`
	PersonPub string    `json:"personPub"`
	At        time.Time `json:"at"`
	By        string    `json:"by,omitempty"`
	PrevSig   string    `json:"prevSig,omitempty"`
}

// RotationBody is what a rotation's prevSig covers (v0.5 §3), the same in
// the private person.key.rotated and a board's member.key.rotated.
type RotationBody struct {
	PersonID    string    `json:"personId"`
	PersonEpoch int       `json:"personEpoch"`
	PersonPub   string    `json:"personPub"`
	By          string    `json:"by"`
	At          time.Time `json:"at"`
}

// Body is the rotation as its prevSig covers it.
func (r KeyRotated) Body() RotationBody {
	return RotationBody{PersonID: r.PersonID, PersonEpoch: r.Epoch, PersonPub: r.PersonPub, By: r.By, At: r.At}
}

// Profile is the person.profile payload.
type Profile struct {
	PersonID     string `json:"personId"`
	Name         string `json:"name"`
	RootDeviceID string `json:"rootDeviceId"`
	PersonPub    string `json:"personPub"`
	PersonSig    string `json:"personSig"` // over canonical of the other fields
}

// profileBody is what a Profile's personSig covers.
type profileBody struct {
	PersonID     string `json:"personId"`
	Name         string `json:"name"`
	RootDeviceID string `json:"rootDeviceId"`
	PersonPub    string `json:"personPub"`
}

// Canonical is the JSON encoding of v with object keys sorted and no
// spaces (§2). It is used only for person-signed objects; rows are signed
// over their stored bytes.
func Canonical(v any) ([]byte, error) {
	raw, err := json.Marshal(v)
	if err != nil {
		return nil, err
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	var generic any
	if err := dec.Decode(&generic); err != nil {
		return nil, err
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(generic); err != nil { // maps encode with sorted keys
		return nil, err
	}
	return bytes.TrimRight(buf.Bytes(), "\n"), nil
}

// verifyPerson checks an ed25519 signature (hex) by pub (hex) over canonical(v).
func verifyPerson(pubHex, sigHex string, v any) error {
	pub, err := hex.DecodeString(pubHex)
	if err != nil || len(pub) != ed25519.PublicKeySize {
		return fmt.Errorf("identity: bad person key")
	}
	sig, err := hex.DecodeString(sigHex)
	if err != nil {
		return fmt.Errorf("identity: bad person signature")
	}
	msg, err := Canonical(v)
	if err != nil {
		return err
	}
	if !ed25519.Verify(pub, msg, sig) {
		return fmt.Errorf("identity: person signature does not verify")
	}
	return nil
}

// PersonIDOf is a person's id: the first 16 bytes of sha256 over its first
// public key, hex. It never changes, even when the key rotates.
func PersonIDOf(pub ed25519.PublicKey) string {
	sum := sha256.Sum256(pub)
	return hex.EncodeToString(sum[:16])
}

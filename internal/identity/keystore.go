package identity

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
)

// Keystore seals the person and device secrets at rest (decision 30, D2):
// DPAPI on Windows, a Keychain-held key on macOS, and a plain 0600 file
// elsewhere. Every OS writes the same wrapped-file shape, naming the
// keystore, so a data dir copied to another machine fails plainly instead
// of yielding garbage.
type Keystore interface {
	// Name is written into each wrapped file ("dpapi", "keychain", "file").
	Name() string
	Seal(plain []byte) ([]byte, error)
	Unseal(blob []byte) ([]byte, error)
}

// FileKeystore keeps secrets as they are, protected only by the file's
// 0600 mode. It is the fallback off Windows and macOS, and what tests use.
type FileKeystore struct{}

func (FileKeystore) Name() string                       { return "file" }
func (FileKeystore) Seal(plain []byte) ([]byte, error)  { return append([]byte(nil), plain...), nil }
func (FileKeystore) Unseal(blob []byte) ([]byte, error) { return append([]byte(nil), blob...), nil }

// wrapped is the on-disk shape of a sealed secret.
type wrapped struct {
	V        int    `json:"v"`
	Keystore string `json:"keystore"`
	Blob     []byte `json:"blob"` // base64 in JSON
}

// ErrWrongKeystore: a secret was sealed by another keystore (or another
// machine's), usually because the data dir was copied.
var ErrWrongKeystore = errors.New("identity: key sealed by a different keystore")

func seal(ks Keystore, plain []byte) (wrapped, error) {
	blob, err := ks.Seal(plain)
	if err != nil {
		return wrapped{}, fmt.Errorf("identity: seal with %s: %w", ks.Name(), err)
	}
	return wrapped{V: 1, Keystore: ks.Name(), Blob: blob}, nil
}

func unseal(ks Keystore, w wrapped) ([]byte, error) {
	if w.Keystore != ks.Name() {
		return nil, fmt.Errorf("%w: sealed with %q, this device uses %q (was the data dir copied from another machine?)", ErrWrongKeystore, w.Keystore, ks.Name())
	}
	plain, err := ks.Unseal(w.Blob)
	if err != nil {
		return nil, fmt.Errorf("identity: unseal with %s (was the data dir copied from another machine or user?): %w", ks.Name(), err)
	}
	return plain, nil
}

// writeSealed writes a wrapped secret to path (0600) atomically.
func writeSealed(path string, w wrapped) error {
	out, err := json.MarshalIndent(w, "", "  ")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, out, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

// aesKeystore seals with AES-256-GCM under a key-encryption key that lives
// somewhere safer than the data dir (the macOS Keychain).
type aesKeystore struct {
	name string
	kek  func() ([]byte, error) // 32 bytes
}

func (a aesKeystore) Name() string { return a.name }

func (a aesKeystore) gcm() (cipher.AEAD, error) {
	k, err := a.kek()
	if err != nil {
		return nil, err
	}
	if len(k) != 32 {
		return nil, fmt.Errorf("key-encryption key is %d bytes, want 32", len(k))
	}
	blk, err := aes.NewCipher(k)
	if err != nil {
		return nil, err
	}
	return cipher.NewGCM(blk)
}

func (a aesKeystore) Seal(plain []byte) ([]byte, error) {
	g, err := a.gcm()
	if err != nil {
		return nil, err
	}
	nonce := make([]byte, g.NonceSize())
	if _, err := rand.Read(nonce); err != nil {
		return nil, err
	}
	return g.Seal(nonce, nonce, plain, []byte("irori/key/1")), nil
}

func (a aesKeystore) Unseal(blob []byte) ([]byte, error) {
	g, err := a.gcm()
	if err != nil {
		return nil, err
	}
	if len(blob) < g.NonceSize() {
		return nil, errors.New("sealed key is truncated")
	}
	return g.Open(nil, blob[:g.NonceSize()], blob[g.NonceSize():], []byte("irori/key/1"))
}

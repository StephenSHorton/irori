//go:build darwin

package identity

import (
	"bytes"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"os/exec"
	"strings"
	"sync"
)

// DefaultKeystore on macOS keeps a random 32-byte key-encryption key in the
// login Keychain (one item per data dir) and seals the secrets with it
// (AES-256-GCM). It talks to the Keychain through /usr/bin/security, so the
// build needs no cgo.
func DefaultKeystore(dataDir string) Keystore {
	sum := sha256.Sum256([]byte(dataDir))
	k := &keychainKEK{account: "kek-" + hex.EncodeToString(sum[:8])}
	return aesKeystore{name: "keychain", kek: k.get}
}

const keychainService = "Irori"

type keychainKEK struct {
	account string
	once    sync.Once
	key     []byte
	err     error
}

func (k *keychainKEK) get() ([]byte, error) {
	k.once.Do(func() { k.key, k.err = k.load() })
	return k.key, k.err
}

func (k *keychainKEK) load() ([]byte, error) {
	out, err := exec.Command("/usr/bin/security", "find-generic-password",
		"-s", keychainService, "-a", k.account, "-w").Output()
	if err == nil {
		key, derr := hex.DecodeString(strings.TrimSpace(string(out)))
		if derr != nil || len(key) != 32 {
			return nil, errors.New("identity: the Keychain item for Irori's key is damaged")
		}
		return key, nil
	}
	var ee *exec.ExitError
	// 44 is errSecItemNotFound: make the key on first run.
	if !errors.As(err, &ee) || ee.ExitCode() != 44 {
		return nil, fmt.Errorf("identity: read Irori's key from the Keychain: %w", err)
	}
	key := make([]byte, 32)
	if _, err := rand.Read(key); err != nil {
		return nil, err
	}
	cmd := exec.Command("/usr/bin/security", "add-generic-password",
		"-s", keychainService, "-a", k.account, "-l", "Irori device key", "-w", hex.EncodeToString(key))
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return nil, fmt.Errorf("identity: store Irori's key in the Keychain: %w: %s", err, strings.TrimSpace(stderr.String()))
	}
	return key, nil
}

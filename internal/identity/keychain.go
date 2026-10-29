package identity

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
)

// The macOS keystore's key-encryption key handling, kept free of build tags
// so it is tested on every OS; keystore_darwin.go wires it to
// /usr/bin/security.

const keychainService = "Irori"

// security exit codes the KEK handling acts on.
const (
	errSecItemNotFound  = 44
	errSecDuplicateItem = 45
)

// errKeystoreUnavailable marks a keystore that could not produce its key,
// as opposed to a sealed blob that doesn't open: unseal doesn't blame a
// copied data dir for it.
var errKeystoreUnavailable = errors.New("identity: the keystore's key is unavailable")

// ErrKEKMissing: the Keychain holds no key for this data dir, but secrets
// here were sealed with one. A new key could never open them, so none is
// made.
var ErrKEKMissing = errors.New("identity: Irori's key is missing from the Keychain")

// keychainKEK is one data dir's 32-byte key-encryption key in the login
// Keychain. A key is made only on first run: while a file in the data dir
// is sealed with the keychain keystore, a missing item is an error.
type keychainKEK struct {
	dataDir  string
	accounts []string // the item's account names, the one to create first
	// run runs /usr/bin/security with args and returns its stdout and exit
	// code (-1 with err when it didn't run).
	run func(args ...string) (out []byte, code int, err error)

	mu  sync.Mutex
	key []byte // kept once read or made; a failure is retried next call
}

func newKeychainKEK(dataDir string, run func(args ...string) ([]byte, int, error)) *keychainKEK {
	return &keychainKEK{dataDir: dataDir, accounts: kekAccounts(dataDir), run: run}
}

// kekAccounts names the Keychain item after the data dir's absolute, clean
// path, so another spelling of the same dir finds the same item. Builds
// before that hashed the path as given; that name is tried second, so an
// item they made is still found.
func kekAccounts(dataDir string) []string {
	name := func(p string) string {
		sum := sha256.Sum256([]byte(p))
		return "kek-" + hex.EncodeToString(sum[:8])
	}
	canon := dataDir
	if abs, err := filepath.Abs(dataDir); err == nil {
		canon = abs
	}
	canon = filepath.Clean(canon)
	out := []string{name(canon)}
	if legacy := name(dataDir); legacy != out[0] {
		out = append(out, legacy)
	}
	return out
}

func (k *keychainKEK) get() ([]byte, error) {
	k.mu.Lock()
	defer k.mu.Unlock()
	if k.key != nil {
		return k.key, nil
	}
	key, err := k.load()
	if err != nil {
		return nil, fmt.Errorf("%w: %w", errKeystoreUnavailable, err)
	}
	k.key = key
	return key, nil
}

func (k *keychainKEK) load() ([]byte, error) {
	if key, err := k.find(); !errors.Is(err, errNotFound) {
		return key, err
	}
	if sealed := keychainSealed(k.dataDir); len(sealed) > 0 {
		return nil, fmt.Errorf("%w: %s in %s %s sealed with it. The Keychain item (service %q, account %q) was deleted, or the data dir was moved or renamed (the item is named after its path). Restore the item from a Keychain backup, or move the data dir back",
			ErrKEKMissing, strings.Join(sealed, " and "), k.dataDir, isAre(len(sealed)), keychainService, k.accounts[0])
	}
	key := make([]byte, 32)
	if _, err := rand.Read(key); err != nil {
		return nil, err
	}
	_, code, err := k.run("add-generic-password", "-s", keychainService, "-a", k.accounts[0], "-l", "Irori device key", "-w", hex.EncodeToString(key))
	switch {
	case err == nil:
		return key, nil
	case code == errSecDuplicateItem:
		// Another process made it between the find and the add: use theirs.
		return k.find()
	default:
		return nil, fmt.Errorf("identity: store Irori's key in the Keychain: %w", err)
	}
}

var errNotFound = errors.New("not found")

// find reads the item under any of its account names.
func (k *keychainKEK) find() ([]byte, error) {
	for _, acct := range k.accounts {
		out, code, err := k.run("find-generic-password", "-s", keychainService, "-a", acct, "-w")
		if err != nil {
			if code == errSecItemNotFound {
				continue
			}
			return nil, fmt.Errorf("identity: read Irori's key from the Keychain: %w", err)
		}
		key, derr := hex.DecodeString(strings.TrimSpace(string(out)))
		if derr != nil || len(key) != 32 {
			return nil, fmt.Errorf("identity: the Keychain item for Irori's key (account %q) is damaged", acct)
		}
		return key, nil
	}
	return nil, errNotFound
}

func isAre(n int) string {
	if n == 1 {
		return "is"
	}
	return "are"
}

// keychainSealed lists the files in dataDir holding a secret sealed with
// the keychain keystore: person.json and the *.key files (device.key, a
// home server's turn.key).
func keychainSealed(dataDir string) []string {
	var out []string
	if raw, err := os.ReadFile(filepath.Join(dataDir, PersonFile)); err == nil {
		var f personFile
		if json.Unmarshal(raw, &f) == nil && f.SecretKey.Keystore == "keychain" {
			out = append(out, PersonFile)
		}
	}
	keys, _ := filepath.Glob(filepath.Join(dataDir, "*.key"))
	for _, p := range keys {
		raw, err := os.ReadFile(p)
		if err != nil {
			continue
		}
		var w wrapped
		if json.Unmarshal(raw, &w) == nil && w.Keystore == "keychain" {
			out = append(out, filepath.Base(p))
		}
	}
	return out
}

package identity

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/StephenSHorton/irori/internal/device"
)

// fakeSecurity is /usr/bin/security over an in-memory Keychain.
type fakeSecurity struct {
	mu    sync.Mutex
	items map[string]string // account -> password
	calls []string
	// raceAdd makes the next add find an item another process just made.
	raceAdd string
	// fail makes every call fail with this exit code (a locked Keychain).
	fail int
}

func newFakeSecurity() *fakeSecurity { return &fakeSecurity{items: map[string]string{}} }

func (f *fakeSecurity) run(args ...string) ([]byte, int, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.calls = append(f.calls, args[0])
	if f.fail != 0 {
		return nil, f.fail, fmt.Errorf("exit status %d", f.fail)
	}
	flag := func(name string) string {
		for i := 0; i+1 < len(args); i++ {
			if args[i] == name {
				return args[i+1]
			}
		}
		return ""
	}
	acct := flag("-a")
	switch args[0] {
	case "find-generic-password":
		pw, ok := f.items[acct]
		if !ok {
			return nil, errSecItemNotFound, errors.New("exit status 44")
		}
		return []byte(pw + "\n"), 0, nil
	case "add-generic-password":
		if f.raceAdd != "" {
			f.items[acct], f.raceAdd = f.raceAdd, ""
		}
		if _, ok := f.items[acct]; ok {
			return nil, errSecDuplicateItem, errors.New("exit status 45")
		}
		f.items[acct] = flag("-w")
		return nil, 0, nil
	}
	return nil, 1, errors.New("unknown command")
}

func keychainStore(dir string, sec *fakeSecurity) Keystore {
	return aesKeystore{name: "keychain", kek: newKeychainKEK(dir, sec.run).get}
}

// First run makes the key; a restart (a new keystore on the same dir)
// loads it and opens what the first run sealed.
func TestKeychainKEKMadeOnceThenLoaded(t *testing.T) {
	dir, sec := t.TempDir(), newFakeSecurity()
	dev, err := device.LoadOrCreate(dir)
	if err != nil {
		t.Fatal(err)
	}
	a, err := Load(dir, dev, keychainStore(dir, sec))
	if err != nil {
		t.Fatal(err)
	}
	if len(sec.items) != 1 {
		t.Fatalf("items after first run: %d", len(sec.items))
	}
	b, err := Load(dir, dev, keychainStore(dir, sec))
	if err != nil {
		t.Fatalf("restart: %v", err)
	}
	if a.Person.ID != b.Person.ID || !a.DevicePub().Equal(b.DevicePub()) || len(sec.items) != 1 {
		t.Fatal("restart made new keys")
	}
	// Another spelling of the same dir finds the same item.
	other := dir + string(filepath.Separator) + "." + string(filepath.Separator)
	if _, err := Load(other, dev, keychainStore(other, sec)); err != nil || len(sec.items) != 1 {
		t.Fatalf("same dir spelled %q: %v, %d items", other, err, len(sec.items))
	}
}

// With secrets sealed under the keychain and its item gone, no new key is
// made: the error says the item is missing, not that the dir was copied.
func TestKeychainKEKMissingIsNotRemade(t *testing.T) {
	dir, sec := t.TempDir(), newFakeSecurity()
	dev, _ := device.LoadOrCreate(dir)
	if _, err := Load(dir, dev, keychainStore(dir, sec)); err != nil {
		t.Fatal(err)
	}
	sec.items = map[string]string{} // the item was deleted
	_, err := Load(dir, dev, keychainStore(dir, sec))
	if !errors.Is(err, ErrKEKMissing) {
		t.Fatalf("err = %v, want ErrKEKMissing", err)
	}
	if strings.Contains(err.Error(), "copied") || !strings.Contains(err.Error(), DeviceKeyFile) || !strings.Contains(err.Error(), PersonFile) {
		t.Fatalf("misleading error: %v", err)
	}
	if len(sec.items) != 0 {
		t.Fatal("a new key was stored while old secrets need the old one")
	}
}

// Moving the data dir is the same case: its path names the item, so the
// moved dir finds none, and no key is made for it.
func TestKeychainKEKMovedDir(t *testing.T) {
	dir, sec := t.TempDir(), newFakeSecurity()
	dev, _ := device.LoadOrCreate(dir)
	if _, err := Load(dir, dev, keychainStore(dir, sec)); err != nil {
		t.Fatal(err)
	}
	moved := filepath.Join(t.TempDir(), "moved")
	if err := os.Rename(dir, moved); err != nil {
		t.Fatal(err)
	}
	if _, err := Load(moved, dev, keychainStore(moved, sec)); !errors.Is(err, ErrKEKMissing) {
		t.Fatalf("moved dir: err = %v", err)
	}
	if len(sec.items) != 1 {
		t.Fatal("a key was made for the moved dir")
	}
}

// An item an earlier build made under the raw path's name is still found.
func TestKeychainKEKFindsTheLegacyName(t *testing.T) {
	dir := t.TempDir()
	rel, err := filepath.Rel(mustWd(t), dir)
	if err != nil {
		t.Skip("no relative path to the temp dir")
	}
	accts := kekAccounts(rel)
	if len(accts) != 2 {
		t.Fatalf("a relative path should have two names: %v", accts)
	}
	sec := newFakeSecurity()
	sec.items[accts[1]] = strings.Repeat("ab", 32)
	k := newKeychainKEK(rel, sec.run)
	key, err := k.get()
	if err != nil || len(key) != 32 || len(sec.items) != 1 {
		t.Fatalf("legacy item: %v, %d items", err, len(sec.items))
	}
}

func mustWd(t *testing.T) string {
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	return wd
}

// Two processes making the key at once: the loser's add finds the
// winner's item and uses it.
func TestKeychainKEKAddRace(t *testing.T) {
	dir, sec := t.TempDir(), newFakeSecurity()
	theirs := strings.Repeat("cd", 32)
	sec.raceAdd = theirs
	key, err := newKeychainKEK(dir, sec.run).get()
	if err != nil {
		t.Fatal(err)
	}
	if fmt.Sprintf("%x", key) != theirs {
		t.Fatal("the loser kept its own key")
	}
}

// A Keychain that fails (locked, access denied) is an error, not a new
// key, and the next call tries again instead of keeping the failure.
func TestKeychainKEKFailureIsRetried(t *testing.T) {
	dir, sec := t.TempDir(), newFakeSecurity()
	sec.fail = 51
	k := newKeychainKEK(dir, sec.run)
	if _, err := k.get(); err == nil || !errors.Is(err, errKeystoreUnavailable) {
		t.Fatalf("locked Keychain: err = %v", err)
	}
	if len(sec.items) != 0 {
		t.Fatal("made a key while the Keychain failed")
	}
	sec.fail = 0
	if _, err := k.get(); err != nil {
		t.Fatalf("after the Keychain came back: %v", err)
	}
	// The unseal error names the keystore's key, not a copied dir.
	ks := aesKeystore{name: "keychain", kek: newKeychainKEK(dir, (&fakeSecurity{fail: 51}).run).get}
	_, err := unseal(ks, wrapped{V: 1, Keystore: "keychain", Blob: make([]byte, 40)})
	if err == nil || strings.Contains(err.Error(), "copied") {
		t.Fatalf("unseal with an unavailable key: %v", err)
	}
}

package identity

import (
	"crypto/hkdf"
	"crypto/rand"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"sort"
	"sync"
)

// A board's secret (design/v0.5-shared-boards.md §6.1): 32 random bytes
// per board epoch, held by every member device and never in a row. It is
// the board's go-room room key: it seals the board's signaling and names
// its topic. This file keeps a board's secrets on this device, sealed by
// the device's Keystore beside the board's log (boards/<boardId>.secrets).
// Handing them to other member devices in board.secret frames is the
// transport's (§12 step 5); two secrets for one epoch, from two devices
// that rotated while apart, settle on the lower maker's device id.

// BoardSecretSize is a board secret's length.
const BoardSecretSize = 32

// BoardTopic is one epoch's board topic: its name and its sealing key.
type BoardTopic struct {
	Epoch int
	Topic string
	Key   [32]byte
}

// DeriveBoardTopic derives epoch's topic from its secret (§6.1):
// HKDF-SHA256 with salt "irori/board/1" and info the boardId's 16 bytes
// then the epoch as 8 bytes big-endian, 48 bytes out: the sealing key,
// then the topic suffix.
func DeriveBoardTopic(boardID string, epoch int, secret []byte) (BoardTopic, error) {
	id, err := hex.DecodeString(boardID)
	if err != nil || len(id) != 16 {
		return BoardTopic{}, fmt.Errorf("identity: %q is not a board id", boardID)
	}
	if epoch < 1 || len(secret) != BoardSecretSize {
		return BoardTopic{}, errors.New("identity: a board topic needs an epoch and a 32-byte secret")
	}
	info := binary.BigEndian.AppendUint64(id, uint64(epoch))
	out, err := hkdf.Key(sha256.New, secret, []byte("irori/board/1"), string(info), 48)
	if err != nil {
		return BoardTopic{}, err
	}
	t := BoardTopic{Epoch: epoch, Topic: "irori/board/1/" + hex.EncodeToString(out[32:])}
	copy(t.Key[:], out[:32])
	return t, nil
}

// BoardSecrets is one board's secrets on this device.
type BoardSecrets struct {
	mu      sync.Mutex
	path    string
	boardID string
	ks      Keystore
	f       secretFile
}

// secretFile is what the sealed file holds.
type secretFile struct {
	// Epochs: epoch -> maker deviceId -> secret.
	Epochs map[int]map[string][]byte `json:"epochs"`
	// Settled is the newest epoch every member device has said hello
	// with; the topic before it is closed.
	Settled int `json:"settled"`
}

// Keystore is the keystore this identity seals its secrets with.
func (id *Identity) Keystore() Keystore { return id.ks }

// OpenBoardSecrets reads boardID's secrets from path (none yet when the
// file doesn't exist).
func OpenBoardSecrets(path, boardID string, ks Keystore) (*BoardSecrets, error) {
	if ks == nil {
		ks = FileKeystore{}
	}
	s := &BoardSecrets{path: path, boardID: boardID, ks: ks, f: secretFile{Epochs: map[int]map[string][]byte{}}}
	raw, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return s, nil
	}
	if err != nil {
		return nil, err
	}
	var w wrapped
	if err := json.Unmarshal(raw, &w); err != nil {
		return nil, fmt.Errorf("identity: %s: %w", path, err)
	}
	plain, err := unseal(ks, w)
	if err != nil {
		return nil, err
	}
	if err := json.Unmarshal(plain, &s.f); err != nil {
		return nil, fmt.Errorf("identity: %s: %w", path, err)
	}
	if s.f.Epochs == nil {
		s.f.Epochs = map[int]map[string][]byte{}
	}
	return s, nil
}

// Epoch is the newest epoch this device holds a secret for (0 for none).
func (s *BoardSecrets) Epoch() int {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.newest()
}

func (s *BoardSecrets) newest() int {
	n := 0
	for e := range s.f.Epochs {
		n = max(n, e)
	}
	return n
}

// Mint makes this device's secret for epoch (a rotation, §6.1). It is a
// no-op when maker already made one for it.
func (s *BoardSecrets) Mint(epoch int, maker string) error {
	var b [BoardSecretSize]byte
	if _, err := rand.Read(b[:]); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if _, ok := s.f.Epochs[epoch][maker]; ok {
		return nil
	}
	return s.addLocked(epoch, maker, b[:])
}

// Add keeps a secret another member device made for epoch (from a
// board.secret frame, §6.1).
func (s *BoardSecrets) Add(epoch int, maker string, secret []byte) error {
	if len(secret) != BoardSecretSize {
		return errors.New("identity: a board secret is 32 bytes")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if old, ok := s.f.Epochs[epoch][maker]; ok && string(old) == string(secret) {
		return nil
	}
	return s.addLocked(epoch, maker, secret)
}

func (s *BoardSecrets) addLocked(epoch int, maker string, secret []byte) error {
	if epoch < 1 || maker == "" {
		return errors.New("identity: a board secret needs an epoch and its maker")
	}
	if s.f.Epochs[epoch] == nil {
		s.f.Epochs[epoch] = map[string][]byte{}
	}
	s.f.Epochs[epoch][maker] = append([]byte(nil), secret...)
	// Only the newest epoch and the one before it stay (§6.1).
	cur := s.newest()
	prev := s.prevLocked(cur)
	for e := range s.f.Epochs {
		if e < cur && e != prev {
			delete(s.f.Epochs, e)
		}
	}
	return s.saveLocked()
}

// prevLocked is the newest held epoch below cur (0 for none).
func (s *BoardSecrets) prevLocked(cur int) int {
	p := 0
	for e := range s.f.Epochs {
		if e < cur {
			p = max(p, e)
		}
	}
	return p
}

// Secret is epoch's secret and its maker: of two makers, the lower device
// id's.
func (s *BoardSecrets) Secret(epoch int) ([]byte, string, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.secretLocked(epoch)
}

func (s *BoardSecrets) secretLocked(epoch int) ([]byte, string, bool) {
	makers := make([]string, 0, len(s.f.Epochs[epoch]))
	for m := range s.f.Epochs[epoch] {
		makers = append(makers, m)
	}
	if len(makers) == 0 {
		return nil, "", false
	}
	sort.Strings(makers)
	return append([]byte(nil), s.f.Epochs[epoch][makers[0]]...), makers[0], true
}

// Topics are the board topics this device keeps open: the newest epoch's,
// and the one before it until Settle says every member device has the
// newest (§6.1).
func (s *BoardSecrets) Topics() ([]BoardTopic, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	cur := s.newest()
	if cur == 0 {
		return nil, nil
	}
	epochs := []int{cur}
	if prev := s.prevLocked(cur); prev > 0 && s.f.Settled < cur {
		epochs = append(epochs, prev)
	}
	var out []BoardTopic
	for _, e := range epochs {
		secret, _, _ := s.secretLocked(e)
		t, err := DeriveBoardTopic(s.boardID, e, secret)
		if err != nil {
			return nil, err
		}
		out = append(out, t)
	}
	return out, nil
}

// Settle records that every member device has said hello with epoch, so
// the topic before it closes.
func (s *BoardSecrets) Settle(epoch int) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if epoch <= s.f.Settled {
		return nil
	}
	s.f.Settled = epoch
	for e := range s.f.Epochs {
		if e < epoch {
			delete(s.f.Epochs, e)
		}
	}
	return s.saveLocked()
}

func (s *BoardSecrets) saveLocked() error {
	plain, err := json.Marshal(s.f)
	if err != nil {
		return err
	}
	w, err := seal(s.ks, plain)
	if err != nil {
		return err
	}
	return writeSealed(s.path, w)
}

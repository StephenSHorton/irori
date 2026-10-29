package identity

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const testBoardID = "0123456789abcdef0123456789abcdef"

// TestBoardTopicDerivation: one secret and epoch give one topic and key;
// another epoch, board or secret gives another (§6.1).
func TestBoardTopicDerivation(t *testing.T) {
	sec := bytes.Repeat([]byte{7}, BoardSecretSize)
	a, err := DeriveBoardTopic(testBoardID, 1, sec)
	if err != nil {
		t.Fatal(err)
	}
	again, _ := DeriveBoardTopic(testBoardID, 1, sec)
	if a != again || !strings.HasPrefix(a.Topic, "irori/board/1/") || len(a.Topic) != len("irori/board/1/")+32 {
		t.Fatalf("topic %q", a.Topic)
	}
	other := bytes.Repeat([]byte{8}, BoardSecretSize)
	for name, alt := range map[string]func() (BoardTopic, error){
		"epoch":  func() (BoardTopic, error) { return DeriveBoardTopic(testBoardID, 2, sec) },
		"board":  func() (BoardTopic, error) { return DeriveBoardTopic("fedcba9876543210fedcba9876543210", 1, sec) },
		"secret": func() (BoardTopic, error) { return DeriveBoardTopic(testBoardID, 1, other) },
	} {
		b, err := alt()
		if err != nil {
			t.Fatal(err)
		}
		if b.Topic == a.Topic || b.Key == a.Key {
			t.Errorf("another %s gives the same topic or key", name)
		}
	}
	if _, err := DeriveBoardTopic("nope", 1, sec); err == nil {
		t.Error("a bad board id derived a topic")
	}
	if _, err := DeriveBoardTopic(testBoardID, 1, sec[:5]); err == nil {
		t.Error("a short secret derived a topic")
	}
}

// TestBoardSecretsRotate: a rotation gives a new topic, the previous stays
// open until every member device has the new epoch, two makers of one
// epoch settle on the lower device id, and the file keeps it all sealed.
func TestBoardSecretsRotate(t *testing.T) {
	path := filepath.Join(t.TempDir(), testBoardID+".secrets")
	s, err := OpenBoardSecrets(path, testBoardID, FileKeystore{})
	if err != nil {
		t.Fatal(err)
	}
	if s.Epoch() != 0 {
		t.Fatal("a new board holds a secret")
	}
	if err := s.Mint(1, "pc"); err != nil {
		t.Fatal(err)
	}
	one, _, _ := s.Secret(1)
	if err := s.Mint(1, "pc"); err != nil {
		t.Fatal(err)
	}
	if again, _, _ := s.Secret(1); !bytes.Equal(one, again) {
		t.Fatal("minting an epoch twice replaced its secret")
	}
	ts, _ := s.Topics()
	if len(ts) != 1 || ts[0].Epoch != 1 {
		t.Fatalf("topics %+v", ts)
	}
	if err := s.Mint(2, "pc"); err != nil {
		t.Fatal(err)
	}
	ts, _ = s.Topics()
	if len(ts) != 2 || ts[0].Epoch != 2 || ts[1].Epoch != 1 || ts[0].Topic == ts[1].Topic {
		t.Fatalf("after a rotation the topics are %+v, want epoch 2 then 1", ts)
	}
	// Another device rotated to epoch 2 while apart: the lower id's wins.
	mac := bytes.Repeat([]byte{9}, BoardSecretSize)
	if err := s.Add(2, "aaa-mac", mac); err != nil {
		t.Fatal(err)
	}
	if got, maker, _ := s.Secret(2); maker != "aaa-mac" || !bytes.Equal(got, mac) {
		t.Fatalf("epoch 2's secret is %s's", maker)
	}
	if err := s.Settle(2); err != nil {
		t.Fatal(err)
	}
	if ts, _ = s.Topics(); len(ts) != 1 || ts[0].Epoch != 2 {
		t.Fatalf("after every device has epoch 2 the topics are %+v", ts)
	}
	// It reopens the same, and the file is the keystore's wrapping.
	r, err := OpenBoardSecrets(path, testBoardID, FileKeystore{})
	if err != nil {
		t.Fatal(err)
	}
	if got, maker, ok := r.Secret(2); !ok || maker != "aaa-mac" || !bytes.Equal(got, mac) || r.Epoch() != 2 {
		t.Fatal("the secrets didn't survive a reopen")
	}
	if _, _, ok := r.Secret(1); ok {
		t.Fatal("a settled epoch's predecessor is still held")
	}
	raw, _ := os.ReadFile(path)
	if !bytes.Contains(raw, []byte(`"keystore": "file"`)) {
		t.Fatalf("not a wrapped file: %s", raw)
	}
	if _, err := OpenBoardSecrets(path, testBoardID, aesKeystore{name: "other", kek: func() ([]byte, error) { return make([]byte, 32), nil }}); err == nil {
		t.Fatal("another keystore opened the secrets")
	}
}

// TestBoardSecretMakersThatCount: two secrets for one epoch settle on the
// lower maker, but only among makers that count (§6.1): a cut device's
// secret, made for the rotation that shuts it out, never wins the tie.
func TestBoardSecretMakersThatCount(t *testing.T) {
	s, err := OpenBoardSecrets(filepath.Join(t.TempDir(), testBoardID+".secrets"), testBoardID, FileKeystore{})
	if err != nil {
		t.Fatal(err)
	}
	if err := s.Add(2, "aaa-stolen", bytes.Repeat([]byte{1}, BoardSecretSize)); err != nil {
		t.Fatal(err)
	}
	if err := s.Add(2, "pc", bytes.Repeat([]byte{2}, BoardSecretSize)); err != nil {
		t.Fatal(err)
	}
	if _, maker, _ := s.Secret(2); maker != "aaa-stolen" {
		t.Fatalf("with no rule the lower maker wins, got %q", maker)
	}
	s.SetMakers(func(dev string) bool { return dev != "aaa-stolen" })
	if _, maker, ok := s.Secret(2); !ok || maker != "pc" {
		t.Fatalf("a cut device's secret won the tie: %q %v", maker, ok)
	}
	s.SetMakers(func(string) bool { return false })
	if ts, err := s.Topics(); err != nil || len(ts) != 0 {
		t.Fatalf("topics from makers that don't count: %+v %v", ts, err)
	}
}

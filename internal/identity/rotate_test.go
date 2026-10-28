package identity

import (
	"bytes"
	"errors"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/device"
)

func TestRotateKeepsThePreviousEpochAndSurvivesReload(t *testing.T) {
	dir := t.TempDir()
	dev, err := device.LoadOrCreate(dir)
	if err != nil {
		t.Fatal(err)
	}
	id, err := Load(dir, dev, FileKeystore{})
	if err != nil {
		t.Fatal(err)
	}
	first := id.Current()
	seed1, ok := id.PersonSeed(1)
	if !ok || id.PrevEpoch() != 0 {
		t.Fatal("a new person has no epoch-1 seed, or already has a previous epoch")
	}
	rot, err := id.Rotate(time.Now())
	if err != nil {
		t.Fatal(err)
	}
	cur := id.Current()
	if rot.Epoch != 2 || cur.Epoch != 2 || rot.PersonPub != cur.Pub || cur.Pub == first.Pub || rot.PersonID != first.ID {
		t.Fatalf("rotated %+v, current %+v", rot, cur)
	}
	if got, ok := id.PersonSeed(1); !ok || !bytes.Equal(got, seed1) || id.PrevEpoch() != 1 {
		t.Fatal("the previous epoch's seed was not kept")
	}
	// New certs are signed with the new key.
	add, err := id.CertFor("dev-b", first.Pub, "b", RoleDesktop, time.Now())
	if err != nil || add.PersonPub != cur.Pub || verifyPerson(add.PersonPub, add.PersonSig, add.Cert) != nil {
		t.Fatalf("cert after rotation: %+v, %v", add, err)
	}
	re, err := Load(dir, id.Device, FileKeystore{})
	if err != nil {
		t.Fatal(err)
	}
	if re.Current() != cur || re.PrevEpoch() != 1 {
		t.Fatalf("reloaded %+v prev %d", re.Current(), re.PrevEpoch())
	}
	seed2, _ := id.PersonSeed(2)
	if got, _ := re.PersonSeed(2); !bytes.Equal(got, seed2) {
		t.Fatal("reload changed the epoch-2 seed")
	}
}

func TestAdoptSecretChecksTheRotationRow(t *testing.T) {
	newID := func() *Identity {
		dir := t.TempDir()
		dev, _ := device.LoadOrCreate(dir)
		id, err := Load(dir, dev, FileKeystore{})
		if err != nil {
			t.Fatal(err)
		}
		return id
	}
	a, b := newID(), newID()
	rot, err := a.Rotate(time.Now())
	if err != nil {
		t.Fatal(err)
	}
	seed, _ := a.PersonSeed(2)
	if err := b.AdoptSecret(2, seed, b.Current().Pub); err == nil {
		t.Fatal("adopted a secret that does not match its rotation row")
	}
	if err := b.AdoptSecret(1, seed, rot.PersonPub); !errors.Is(err, ErrStaleEpoch) {
		t.Fatalf("adopt for the current epoch: %v", err)
	}
	if err := b.AdoptSecret(2, seed, rot.PersonPub); err != nil {
		t.Fatal(err)
	}
	if b.Current().Epoch != 2 || b.Current().Pub != rot.PersonPub || b.PrevEpoch() != 1 {
		t.Fatalf("after adopt: %+v", b.Current())
	}
}

// Every older epoch is kept across rotations and a reload, so a device
// offline through two revokes still finds a topic, until Forget.
func TestEveryOlderEpochIsKeptUntilForget(t *testing.T) {
	dir := t.TempDir()
	dev, _ := device.LoadOrCreate(dir)
	id, err := Load(dir, dev, FileKeystore{})
	if err != nil {
		t.Fatal(err)
	}
	seed1, _ := id.PersonSeed(1)
	for range 2 {
		if _, err := id.Rotate(time.Now()); err != nil {
			t.Fatal(err)
		}
	}
	re, err := Load(dir, id.Device, FileKeystore{})
	if err != nil {
		t.Fatal(err)
	}
	if got, ok := re.PersonSeed(1); !ok || !bytes.Equal(got, seed1) || re.Current().Epoch != 3 || re.PrevEpoch() != 2 {
		t.Fatalf("after two rotations: epoch %d prev %d, epoch 1 kept %v", re.Current().Epoch, re.PrevEpoch(), ok)
	}
	if err := re.Forget(3); err != nil {
		t.Fatal(err)
	}
	re2, _ := Load(dir, id.Device, FileKeystore{})
	if _, ok := re2.PersonSeed(1); ok || len(re2.OlderEpochs()) != 0 {
		t.Fatalf("Forget kept %v", re2.OlderEpochs())
	}
}

// The losing side of two rotations to one epoch takes the winner's key in
// place of its own; anything else is refused.
func TestReplaceSecretSwapsTheCurrentEpochsKey(t *testing.T) {
	newID := func() *Identity {
		dir := t.TempDir()
		dev, _ := device.LoadOrCreate(dir)
		id, err := Load(dir, dev, FileKeystore{})
		if err != nil {
			t.Fatal(err)
		}
		return id
	}
	a, b := newID(), newID()
	ra, _ := a.Rotate(time.Now())
	if _, err := b.Rotate(time.Now()); err != nil {
		t.Fatal(err)
	}
	seed, _ := a.PersonSeed(2)
	if err := b.ReplaceSecret(2, seed, b.Current().Pub); err == nil {
		t.Fatal("replaced with a seed that doesn't match the key named")
	}
	if err := b.ReplaceSecret(3, seed, ra.PersonPub); !errors.Is(err, ErrStaleEpoch) {
		t.Fatalf("replace for another epoch: %v", err)
	}
	if err := b.ReplaceSecret(2, seed, ra.PersonPub); err != nil {
		t.Fatal(err)
	}
	re, _ := Load(b.dataDir, b.Device, FileKeystore{})
	if got, _ := re.PersonSeed(2); re.Current().Pub != ra.PersonPub || !bytes.Equal(got, seed) || re.PrevEpoch() != 1 {
		t.Fatalf("after replace: %+v", re.Current())
	}
}

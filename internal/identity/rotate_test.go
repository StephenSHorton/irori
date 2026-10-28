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

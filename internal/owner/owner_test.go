package owner

import (
	"testing"

	"github.com/StephenSHorton/irori/internal/store"
)

func TestDeviceOfAndMayWrite(t *testing.T) {
	for key, dev := range map[string]string{
		"pc/aaaa-session":       "pc",
		"mac/grok-1":            "mac",
		"user:pc":               "pc",
		"irori":                 "",
		"":                      "",
		"remote:github.com/a/b": "",
	} {
		if got := DeviceOf(key); got != dev {
			t.Errorf("DeviceOf(%q) = %q, want %q", key, got, dev)
		}
	}
	if !MayWrite("pc/s", "pc") || MayWrite("pc/s", "mac") || !MayWrite("user:mac", "mac") || MayWrite("user:mac", "pc") {
		t.Fatal("MayWrite")
	}
	if !MayWrite("irori", "anyone") {
		t.Fatal("a key naming no device has no owner to enforce")
	}
}

func TestTableKindsAreClassifiedAndReplicate(t *testing.T) {
	for _, r := range Table {
		for _, k := range append(append([]string{}, r.OwnerKinds...), r.RequestKinds...) {
			if store.KindSide(k) != store.Replicated {
				t.Errorf("%s (%s) is not a replicated kind", k, r.Entity)
			}
		}
		for _, k := range r.OwnerKinds {
			if e, ok := EntityOf(k); !ok || e != r.Entity {
				t.Errorf("EntityOf(%s) = %q %v", k, e, ok)
			}
		}
	}
	if _, ok := EntityOf("message.sent"); ok {
		t.Error("message.sent is anyone's to write")
	}
}

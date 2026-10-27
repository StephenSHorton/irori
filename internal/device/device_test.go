package device

import "testing"

func TestLoadOrCreateIsStable(t *testing.T) {
	dir := t.TempDir()
	a, err := LoadOrCreate(dir)
	if err != nil {
		t.Fatal(err)
	}
	b, err := LoadOrCreate(dir)
	if err != nil {
		t.Fatal(err)
	}
	if a.ID == "" || a != b {
		t.Fatalf("identity changed: %+v vs %+v", a, b)
	}
}

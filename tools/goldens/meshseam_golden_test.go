// Golden exporter for crates/meshseam (mesh.md §2.4): MeshSecret and
// MeshInfo, the derivation the iroh epoch-open proof keys on.
//
// This file is not part of the Go repo. tools/goldens/export.sh injects it
// into I:internal/mesh with `go test -overlay`. It only runs when
// IRORI_GOLDEN_OUT is set, and writes $IRORI_GOLDEN_OUT/meshseam/secret.json.
package mesh

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

type goldMeshSecret struct {
	Seed     string `json:"seed"`
	PersonID string `json:"personId"`
	Epoch    int    `json:"epoch"`
	Info     string `json:"info"`
	Key      string `json:"key"`
	Topic    string `json:"topic"`
}

func TestGoldenMeshSeam(t *testing.T) {
	out := os.Getenv("IRORI_GOLDEN_OUT")
	if out == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	seeds := [][]byte{
		make([]byte, 32),
		[]byte("0123456789abcdef0123456789abcdef"),
		{0xff, 0xfe, 0xfd, 0x00, 0x01, 0x02, 0x80, 0x7f, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80,
			0x90, 0xa0, 0xb0, 0xc0, 0xd0, 0xe0, 0xf0, 0x0f, 0x1f, 0x2f, 0x3f, 0x4f, 0x5f, 0x6f, 0x7f, 0x8f},
	}
	people := []string{"", "p-1", "0123456789abcdef0123456789abcdef", "人"}
	epochs := []int{0, 1, 2, 3, 255, 256, 1 << 40}
	var rows []goldMeshSecret
	for _, s := range seeds {
		for _, p := range people {
			for _, e := range epochs {
				sec := MeshSecret(s, p, e)
				rows = append(rows, goldMeshSecret{
					Seed:     hex.EncodeToString(s),
					PersonID: p,
					Epoch:    e,
					Info:     hex.EncodeToString(MeshInfo(p, e)),
					Key:      hex.EncodeToString(sec.Key[:]),
					Topic:    sec.Topic,
				})
			}
		}
	}
	b, err := json.MarshalIndent(map[string]any{"app": MeshApp, "secrets": rows}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	dir := filepath.Join(out, "meshseam")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "secret.json"), append(b, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}

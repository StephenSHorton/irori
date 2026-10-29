package blob

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestMarkDownloadedWritesZoneIdentifier(t *testing.T) {
	p := filepath.Join(t.TempDir(), "shot.png")
	if err := os.WriteFile(p, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := MarkDownloaded(p); err != nil {
		t.Fatal(err)
	}
	b, err := os.ReadFile(p + ":Zone.Identifier")
	if err != nil || !strings.Contains(string(b), "ZoneId=3") {
		t.Fatalf("zone identifier = %q, %v", b, err)
	}
	if b, _ := os.ReadFile(p); string(b) != "x" {
		t.Fatalf("the file itself = %q", b)
	}
}

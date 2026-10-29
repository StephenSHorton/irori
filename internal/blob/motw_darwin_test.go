package blob

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"golang.org/x/sys/unix"
)

func TestMarkDownloadedSetsQuarantine(t *testing.T) {
	p := filepath.Join(t.TempDir(), "shot.png")
	if err := os.WriteFile(p, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := MarkDownloaded(p); err != nil {
		t.Fatal(err)
	}
	buf := make([]byte, 256)
	n, err := unix.Getxattr(p, quarantineAttr, buf)
	if err != nil || !strings.HasPrefix(string(buf[:n]), "0081;") {
		t.Fatalf("quarantine = %q, %v", buf[:max(n, 0)], err)
	}
}

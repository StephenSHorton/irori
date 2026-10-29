package blob

import (
	"fmt"
	"time"

	"golang.org/x/sys/unix"
)

// quarantineAttr is the extended attribute Gatekeeper reads.
const quarantineAttr = "com.apple.quarantine"

// MarkDownloaded marks a file written out of the store as from another
// computer: the quarantine attribute a browser download gets (flags 0081:
// downloaded, not yet approved), so Gatekeeper checks it before it runs.
func MarkDownloaded(path string) error {
	v := fmt.Sprintf("0081;%08x;Irori;", time.Now().Unix())
	return unix.Setxattr(path, quarantineAttr, []byte(v), 0)
}

// Package gitsha computes git object ids for file contents, so capture can
// match an edit's result against the blob a commit stored
// (docs/design/v0.3-capture.md §4, §5).
package gitsha

import (
	"bytes"
	"crypto/sha1"
	"encoding/hex"
	"strconv"
)

// Blob is the git blob id (SHA-1, lowercase hex) of data, the value
// `git hash-object` prints for a file holding exactly these bytes.
func Blob(data []byte) string {
	h := sha1.New()
	h.Write([]byte("blob " + strconv.Itoa(len(data)) + "\x00"))
	h.Write(data)
	return hex.EncodeToString(h.Sum(nil))
}

// LF returns data with every CRLF turned into LF: the form core.autocrlf
// commits on Windows. It returns data itself when there is no CRLF.
func LF(data []byte) []byte {
	if !bytes.Contains(data, []byte("\r\n")) {
		return data
	}
	return bytes.ReplaceAll(data, []byte("\r\n"), []byte("\n"))
}

// Pair is Blob(data) and, when it differs, Blob(LF(data)); lf is "" when
// data has no CRLF.
func Pair(data []byte) (raw, lf string) {
	raw = Blob(data)
	if l := LF(data); len(l) != len(data) {
		lf = Blob(l)
	}
	return raw, lf
}

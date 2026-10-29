//go:build darwin

package identity

import (
	"bytes"
	"errors"
	"fmt"
	"os/exec"
	"strings"
)

// DefaultKeystore on macOS keeps a random 32-byte key-encryption key in the
// login Keychain (one item per data dir) and seals the secrets with it
// (AES-256-GCM). It talks to the Keychain through /usr/bin/security, so the
// build needs no cgo. The key handling is in keychain.go.
func DefaultKeystore(dataDir string) Keystore {
	return aesKeystore{name: "keychain", kek: newKeychainKEK(dataDir, runSecurity).get}
}

func runSecurity(args ...string) ([]byte, int, error) {
	cmd := exec.Command("/usr/bin/security", args...)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err == nil {
		return out, 0, nil
	}
	var ee *exec.ExitError
	if errors.As(err, &ee) {
		return out, ee.ExitCode(), fmt.Errorf("%w: %s", err, strings.TrimSpace(stderr.String()))
	}
	return out, -1, err
}

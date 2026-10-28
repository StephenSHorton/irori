package mesh

import (
	"encoding/json"
	"errors"
	"io/fs"
	"net/url"
	"os"
	"path/filepath"
	"strings"
)

// RelayFile is the relay setting in the data dir. It is device-local
// (§5.1): each device names its own TURN relay, or none.
const RelayFile = "relay.json"

// Relay is the optional TURN relay setting (§4.4): a TURN REST credential
// endpoint and, optionally, TURN URLs that replace the ones it returns.
// The zero value is "Direct only".
type Relay struct {
	CredentialURL string   `json:"credentialUrl,omitempty"`
	TurnURLs      []string `json:"turnUrls,omitempty"`
}

// Direct reports that no relay is set.
func (r Relay) Direct() bool { return r.CredentialURL == "" }

// Check validates a relay setting: the credential URL must be https, except
// on loopback (go-room's rule), and TURN URLs must be turn: or turns:.
func (r Relay) Check() error {
	if r.Direct() {
		if len(r.TurnURLs) > 0 {
			return errors.New("relay: TURN URLs need a credential URL")
		}
		return nil
	}
	u, err := url.Parse(r.CredentialURL)
	if err != nil || u.Host == "" {
		return errors.New("relay: the credential URL is not a URL")
	}
	host := u.Hostname()
	loop := host == "127.0.0.1" || host == "::1" || host == "localhost"
	if u.Scheme != "https" && !(u.Scheme == "http" && loop) {
		return errors.New("relay: the credential URL must be https")
	}
	for _, t := range r.TurnURLs {
		l := strings.ToLower(strings.TrimSpace(t))
		if !strings.HasPrefix(l, "turn:") && !strings.HasPrefix(l, "turns:") {
			return errors.New("relay: TURN URLs start with turn: or turns:")
		}
	}
	return nil
}

// LoadRelay reads the relay setting (none set is the zero value).
func LoadRelay(dataDir string) (Relay, error) {
	raw, err := os.ReadFile(filepath.Join(dataDir, RelayFile))
	if errors.Is(err, fs.ErrNotExist) {
		return Relay{}, nil
	}
	if err != nil {
		return Relay{}, err
	}
	var r Relay
	if err := json.Unmarshal(raw, &r); err != nil {
		return Relay{}, errors.New("relay: relay.json is unreadable")
	}
	return r, nil
}

// SaveRelay checks and writes the relay setting; the zero value removes it.
// It takes effect for connections made after the next net start.
func SaveRelay(dataDir string, r Relay) error {
	if err := r.Check(); err != nil {
		return err
	}
	path := filepath.Join(dataDir, RelayFile)
	if r.Direct() {
		if err := os.Remove(path); err != nil && !errors.Is(err, fs.ErrNotExist) {
			return err
		}
		return nil
	}
	out, _ := json.MarshalIndent(r, "", "  ")
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, out, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

// RelayStatus is how the relay is doing: none (Direct only), pending, ok or
// error, with a reason for error.
type RelayStatus struct {
	State  string `json:"state"`
	Detail string `json:"detail,omitempty"`
}

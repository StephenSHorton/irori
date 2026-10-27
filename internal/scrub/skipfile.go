package scrub

import "strings"

// skipFile matches on the lowercased base name; either slash separates.
func skipFile(path string) bool {
	base := path
	if i := strings.LastIndexAny(base, `/\`); i >= 0 {
		base = base[i+1:]
	}
	base = strings.ToLower(strings.TrimSpace(base))
	if base == "" {
		return false
	}
	switch base {
	case ".env", ".netrc", ".npmrc", ".pypirc":
		return true
	}
	if strings.HasPrefix(base, ".env.") {
		return true
	}
	for _, ext := range []string{".pem", ".key", ".p12", ".pfx"} {
		if strings.HasSuffix(base, ext) {
			return true
		}
	}
	for _, p := range []string{"id_rsa", "id_ed25519", "id_ecdsa", "credentials"} {
		if strings.HasPrefix(base, p) {
			return true
		}
	}
	return false
}

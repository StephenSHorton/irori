package scrub

import "strings"

// skipFile matches on the lowercased base name; either slash separates.
// The base name is first reduced to the file Windows would open: a drive
// prefix (C:.env) and an alternate data stream (.env::$DATA,
// x.pem:Zone.Identifier) are dropped, and so are trailing dots and spaces,
// which Win32 strips (key.pem. opens key.pem).
func skipFile(path string) bool {
	base := path
	if i := strings.LastIndexAny(base, `/\`); i >= 0 {
		base = base[i+1:]
	} else if len(base) >= 2 && base[1] == ':' && isLetter(base[0]) {
		base = base[2:]
	}
	if i := strings.IndexByte(base, ':'); i >= 0 {
		base = base[:i]
	}
	base = strings.ToLower(strings.TrimRight(strings.TrimSpace(base), ". "))
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

func isLetter(c byte) bool { return c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' }

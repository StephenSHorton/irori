package attach

import "strings"

// ModAtLeast reports whether mod version v is at least min (AppendSince,
// say). Both are read as three dot-separated ints: a missing part is 0 and
// whatever follows a part's digits is ignored, so "0.4.0-rc1" is 0.4.0. An
// empty or unreadable v is never at least anything.
func ModAtLeast(v, min string) bool {
	have, ok := parseVersion(v)
	if !ok {
		return false
	}
	want, ok := parseVersion(min)
	if !ok {
		return true
	}
	for i := range have {
		if have[i] != want[i] {
			return have[i] > want[i]
		}
	}
	return true
}

// parseVersion reads "major.minor.patch"; ok is false when the major part
// has no digits.
func parseVersion(v string) ([3]int, bool) {
	var out [3]int
	parts := strings.SplitN(strings.TrimPrefix(strings.TrimSpace(v), "v"), ".", 3)
	for i, p := range parts {
		n, digits := 0, 0
		for _, c := range p {
			if c < '0' || c > '9' || digits == 6 {
				break
			}
			n = n*10 + int(c-'0')
			digits++
		}
		if digits == 0 {
			if i == 0 {
				return out, false
			}
			break
		}
		out[i] = n
		if digits < len(p) {
			break // a suffix ends the version
		}
	}
	return out, true
}

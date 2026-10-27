// Package scrub removes secrets from text before capture stores it
// (docs/design/v0.3-capture.md §9). It runs on every excerpt, tool target,
// summary and patch, before hashing.
//
// Matches become "[redacted:<kind>]". Kinds: anthropic, openai, github, slack,
// aws, google, jwt, private-key, bearer, assignment, exact. Over-redacting a
// harmless string is acceptable; leaking a token is not, but the patterns
// must stay narrow enough that ordinary code (identifiers such as
// tokenCount, `password: string` type annotations, "sk-" in prose) survives.
//
// CONTRACT (implemented by the scrub work package; keep these signatures):
package scrub

// Scrubber is safe for concurrent use.
type Scrubber struct {
	impl *scrubber
}

// New returns a scrubber that also redacts each of exact (kind "exact"),
// e.g. Irori's own lockfile token and attach secrets. Empty and short (<8
// bytes) exact values are ignored.
func New(exact ...string) *Scrubber { return newScrubber(exact) }

// SetExact replaces the exact-value list. On a zero Scrubber the first call
// sets it up, so make that call before sharing it between goroutines.
func (s *Scrubber) SetExact(exact ...string) {
	if s.impl == nil {
		s.impl = &scrubber{}
	}
	s.impl.setExact(exact)
}

// String returns in with every secret replaced. It never grows a line by more
// than the marker, and it keeps line breaks, so patch line counts don't change.
// A nil or zero Scrubber applies the patterns with no exact values.
func (s *Scrubber) String(in string) string {
	if s == nil {
		return (*scrubber)(nil).scrub(in)
	}
	return s.impl.scrub(in)
}

// SkipFile reports whether a path's patch must not be stored at all: base
// names .env and .env.*, *.pem, *.key, *.p12, *.pfx, id_rsa*, id_ed25519*,
// id_ecdsa*, credentials*, .netrc, .npmrc, .pypirc (case-insensitive, either
// slash).
func SkipFile(path string) bool { return skipFile(path) }

package scrub

import (
	"regexp"
	"sort"
	"strings"
	"sync/atomic"
)

// scrubber holds the compiled exact-value replacer; the patterns are shared
// package state.
type scrubber struct {
	exact atomic.Pointer[strings.Replacer] // nil when there are no exact values
}

func newScrubber(exact []string) *Scrubber {
	s := &scrubber{}
	s.setExact(exact)
	return &Scrubber{impl: s}
}

// setExact swaps in a replacer for exact. Values shorter than 8 bytes are
// dropped. A value spanning lines is replaced by the marker plus its line
// breaks, so line counts still hold.
func (s *scrubber) setExact(exact []string) {
	var vals []string
	seen := map[string]bool{}
	for _, v := range exact {
		if len(v) < 8 || seen[v] {
			continue
		}
		seen[v] = true
		vals = append(vals, v)
	}
	if len(vals) == 0 {
		s.exact.Store(nil)
		return
	}
	// Longest first, so a value that contains another is replaced whole.
	sort.Slice(vals, func(i, j int) bool { return len(vals[i]) > len(vals[j]) })
	pairs := make([]string, 0, 2*len(vals))
	for _, v := range vals {
		pairs = append(pairs, v, marker("exact", v))
	}
	s.exact.Store(strings.NewReplacer(pairs...))
}

// scrub runs the exact values first (so Irori's own secrets get kind
// "exact" whatever their shape), then the private-key blocks, then each
// single-line rule in order, line by line.
func (s *scrubber) scrub(in string) string {
	if in == "" {
		return in
	}
	out := in
	if r := s.exact.Load(); r != nil {
		out = r.Replace(out)
	}
	out = scrubPrivateKeys(out)
	return scrubLines(out)
}

// scrubLines runs the rules on each line that passes their gates. Running
// the regexps per line, rather than over the whole text, keeps their inputs
// small, where Go's regexp package is much faster.
func scrubLines(s string) string {
	var b strings.Builder
	changed := false
	rest := s
	for len(rest) > 0 {
		n := strings.IndexByte(rest, '\n')
		line, nl := rest, ""
		if n >= 0 {
			line, nl = rest[:n], rest[n:n+1]
		}
		rest = rest[len(line)+len(nl):]
		got := scrubLine(line)
		if got != line && !changed {
			changed = true
			b.Grow(len(s) + 64)
			b.WriteString(s[:len(s)-len(rest)-len(nl)-len(line)])
		}
		if changed {
			b.WriteString(got)
			b.WriteString(nl)
		}
	}
	if !changed {
		return s
	}
	return b.String()
}

func scrubLine(line string) string {
	for _, r := range rules {
		if r.gate != "" && !strings.Contains(line, r.gate) {
			continue
		}
		if len(r.lowerGates) > 0 && !containsAnyFold(line, r.lowerGates) {
			continue
		}
		line = apply(line, r)
	}
	return line
}

// containsAnyFold reports whether s holds any of words (lowercase ASCII),
// ignoring ASCII case, without allocating.
func containsAnyFold(s string, words []string) bool {
	for i := 0; i < len(s); i++ {
		c := s[i] | 0x20
		for _, w := range words {
			if c != w[0] || len(s)-i < len(w) {
				continue
			}
			j := 1
			for j < len(w) && s[i+j]|0x20 == w[j] || j < len(w) && w[j] == s[i+j] {
				j++
			}
			if j == len(w) {
				return true
			}
		}
	}
	return false
}

// marker is the replacement for match: the kind tag plus every line break in
// match, so the line count never changes.
func marker(kind, match string) string {
	m := "[redacted:" + kind + "]"
	if !strings.ContainsAny(match, "\r\n") {
		return m
	}
	var b strings.Builder
	b.WriteString(m)
	for i := 0; i < len(match); i++ {
		if match[i] == '\r' || match[i] == '\n' {
			b.WriteByte(match[i])
		}
	}
	return b.String()
}

// rule is one single-line pattern. pick returns the span of the match to
// replace (often just the value group) or ok=false to leave it alone.
type rule struct {
	kind       string
	re         *regexp.Regexp
	gate       string   // literal that must appear in the text, case-sensitive
	lowerGates []string // at least one must appear in the lowercased text
	pick       func(s string, m []int) (lo, hi int, ok bool)
}

// whole replaces the entire match.
func whole(s string, m []int) (int, int, bool) { return m[0], m[1], true }

// Every pattern here stays on one line: separators are [ \t], never \s, and
// value classes exclude \r and \n.
var rules = []rule{
	{
		kind: "jwt",
		re:   regexp.MustCompile(`\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]*`),
		gate: "eyJ",
		pick: whole,
	},
	{
		kind: "anthropic",
		re:   regexp.MustCompile(`\bsk-ant-[A-Za-z0-9_-]{20,}`),
		gate: "sk-ant-",
		pick: whole,
	},
	{
		// sk-… with at least one digit: real keys are random, prose like
		// "sk-some-long-hyphenated-words" is not.
		kind: "openai",
		re:   regexp.MustCompile(`\bsk-[A-Za-z0-9_-]{20,}`),
		gate: "sk-",
		pick: func(s string, m []int) (int, int, bool) {
			v := s[m[0]:m[1]]
			return m[0], m[1], strings.ContainsAny(v[3:], "0123456789")
		},
	},
	{
		kind: "github",
		re:   regexp.MustCompile(`\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{22,})`),
		gate: "_",
		pick: whole,
	},
	{
		kind: "slack",
		re:   regexp.MustCompile(`\bxox[abprse]-[A-Za-z0-9-]{10,}`),
		gate: "xox",
		pick: whole,
	},
	{
		kind: "aws",
		re:   regexp.MustCompile(`\b(?:AKIA|ASIA)[A-Z0-9]{16}\b`),
		gate: "A",
		pick: whole,
	},
	{
		kind: "google",
		re:   regexp.MustCompile(`\bAIza[A-Za-z0-9_-]{35}`),
		gate: "AIza",
		pick: whole,
	},
	{
		// Keep the word Bearer, redact the credential. A short or all-letter
		// word ("bearer instruments") is prose.
		kind:       "bearer",
		re:         regexp.MustCompile(`(?i)\bbearer[ \t]+([A-Za-z0-9_\-.~+/]+=*)`),
		lowerGates: []string{"bearer"},
		pick: func(s string, m []int) (int, int, bool) {
			v := s[m[2]:m[3]]
			return m[2], m[3], credential(v)
		},
	},
	{
		// Authorization: Basic|Token|Digest <credential>.
		kind:       "bearer",
		re:         regexp.MustCompile(`(?i)\bauthorization(?:\\?["'])?[ \t]*[:=][ \t]*(?:\\?["'])?(?:basic|token|digest|negotiate)[ \t]+([A-Za-z0-9_\-.~+/]+=*)`),
		lowerGates: []string{"authorization"},
		pick: func(s string, m []int) (int, int, bool) {
			return m[2], m[3], m[3]-m[2] >= 8
		},
	},
	{
		// scheme://user:password@host. Any password length counts here.
		kind: "assignment",
		// The user part may itself be a marker from an earlier rule.
		re:   regexp.MustCompile(`\b[A-Za-z][A-Za-z0-9+.-]*://(?:\[redacted:[a-z-]+\]|[^\s:/@"'<>\[\]]*):([^\s/@"'<>]+)@`),
		gate: "://",
		pick: func(s string, m []int) (int, int, bool) {
			return m[2], m[3], !strings.HasPrefix(s[m[2]:m[3]], "[redacted:")
		},
	},
	{
		kind:       "assignment",
		re:         assignmentRE,
		lowerGates: []string{"token", "secret", "password", "passwd", "apikey", "api_key", "api-key", "authorization"},
		pick:       pickAssignment,
	},
}

// assignmentRE matches key = value, key: value, key := value and key =>
// value, where the key (optionally quoted, as in JSON or escaped JSON)
// contains a secret word. Group 1 is an escaped-quoted value's body (\"…\"
// inside a JSON string), group 2 a quoted value's body (backslash escapes
// allowed), group 3 an unquoted value.
// The match starts at the secret word, not the start of the key: only the
// value is replaced, and a leading [\w.-]* makes the search much slower.
var assignmentRE = regexp.MustCompile(`(?i)(?:token|secret|password|passwd|api[_-]?key|authorization)[A-Za-z0-9_.\-]*(?:\\?["'])?[ \t]*(?::=|=>|[:=])[ \t]*` +
	`(?:\\["']((?:[^"'\\\r\n]|\\[^"'\r\n])+?)\\["']` +
	"|[\"'`]((?:\\\\[^\\r\\n]|[^\"'`\\\\\\r\\n])+)[\"'`]" +
	"|([^\\s\"'`;,&<>(){}\\[\\]\\\\|]+))")

// pickAssignment redacts a value of 8 or more characters, except unquoted
// values that read as code: a call or index (getToken(), cfg[k]), a
// variable reference ($TOKEN, %TOKEN%), or a dotted name
// (process.env.GITHUB_TOKEN).
func pickAssignment(s string, m []int) (int, int, bool) {
	for _, g := range []int{1, 2} {
		if lo, hi := m[2*g], m[2*g+1]; lo >= 0 {
			v := s[lo:hi]
			return lo, hi, len(v) >= 8 && !strings.Contains(v, "[redacted:")
		}
	}
	lo, hi := m[6], m[7]
	if lo < 0 || hi-lo < 8 {
		return 0, 0, false
	}
	v := s[lo:hi]
	if hi < len(s) && (s[hi] == '(' || s[hi] == '[') {
		return 0, 0, false
	}
	switch v[0] {
	case '$', '%', '@', '*':
		return 0, 0, false
	}
	if dottedName.MatchString(v) {
		return 0, 0, false
	}
	return lo, hi, true
}

var dottedName = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)+$`)

// credential reports whether a word after "Bearer" looks like a token: 8+
// characters and either not all letters or long.
func credential(v string) bool {
	if len(v) < 8 {
		return false
	}
	if len(v) >= 16 {
		return true
	}
	for i := 0; i < len(v); i++ {
		c := v[i]
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z') {
			return true
		}
	}
	return false
}

// apply runs one rule over s.
func apply(s string, r rule) string {
	ms := r.re.FindAllStringSubmatchIndex(s, -1)
	if ms == nil {
		return s
	}
	var b strings.Builder
	last := 0
	for _, m := range ms {
		lo, hi, ok := r.pick(s, m)
		if !ok || lo < last || hi <= lo {
			continue
		}
		b.WriteString(s[last:lo])
		b.WriteString(marker(r.kind, s[lo:hi]))
		last = hi
	}
	if last == 0 {
		return s
	}
	b.WriteString(s[last:])
	return b.String()
}

var (
	pemBegin = regexp.MustCompile(`-----BEGIN [A-Z0-9 ]*PRIVATE KEY( BLOCK)?-----`)
	pemEnd   = regexp.MustCompile(`-----END [A-Z0-9 ]*PRIVATE KEY( BLOCK)?-----`)
)

// scrubPrivateKeys replaces each BEGIN … PRIVATE KEY … END block. A block
// with no END (a truncated excerpt) runs to the end of the text. The first
// line of the block becomes the marker; each later line keeps only its
// leading diff/indent prefix, so a patch stays a well-formed patch with the
// same line count.
func scrubPrivateKeys(s string) string {
	if !strings.Contains(s, "PRIVATE KEY") {
		return s
	}
	var b strings.Builder
	last := 0
	for {
		loc := pemBegin.FindStringIndex(s[last:])
		if loc == nil {
			break
		}
		start := last + loc[0]
		end := len(s)
		if e := pemEnd.FindStringIndex(s[last+loc[1]:]); e != nil {
			end = last + loc[1] + e[1]
		}
		b.WriteString(s[last:start])
		b.WriteString("[redacted:private-key]")
		block := s[start:end]
		for {
			i := strings.IndexAny(block, "\r\n")
			if i < 0 {
				break
			}
			j := i + 1
			if block[i] == '\r' && j < len(block) && block[j] == '\n' {
				j++
			}
			b.WriteString(block[i:j])
			block = block[j:]
			b.WriteString(linePrefix(block))
		}
		last = end
	}
	if last == 0 {
		return s
	}
	b.WriteString(s[last:])
	return b.String()
}

// linePrefix is the start of line kept inside a redacted key block: one
// optional diff marker (+, - or space, but not the first dash of
// "-----END") and then any indentation.
func linePrefix(line string) string {
	i := 0
	bareEnd := strings.HasPrefix(line, "-----") && !strings.HasPrefix(line, "------")
	if i < len(line) && (line[i] == '+' || line[i] == ' ' || line[i] == '-' && !bareEnd) {
		i++
	}
	for i < len(line) && (line[i] == ' ' || line[i] == '\t') {
		i++
	}
	return line[:i]
}

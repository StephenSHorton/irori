package coord

import (
	"sort"
	"strings"
	"unicode"
	"unicode/utf8"
)

// Overlap of two POSIX globs: is there a path both can match?
//
// Paths are compared segment by segment. A whole "**" segment matches zero or
// more segments (so "a/**" overlaps "a"); inside a segment "*" matches any run
// of characters, "?" one character and "[...]" one character from a class
// (path.Match syntax, "^" negates; no escapes because globs never contain a
// backslash). A "**" mixed with other characters in one segment is two "*".
//
// The answer leans to true: literals and classes compare case-insensitively
// (Windows and macOS file systems usually are), an unparseable glob overlaps
// everything, and two classes that both contain a letter are assumed to meet.

func overlaps(a, b string) bool {
	na, err := NormalizeGlob(a)
	if err != nil {
		return true
	}
	nb, err := NormalizeGlob(b)
	if err != nil {
		return true
	}
	return overlapsNorm(na, nb)
}

func overlapsNorm(a, b string) bool {
	sa, sb := strings.Split(a, "/"), strings.Split(b, "/")
	ta, ok := tokenizeAll(sa)
	if !ok {
		return true
	}
	tb, ok := tokenizeAll(sb)
	if !ok {
		return true
	}
	memo := map[[2]int]bool{}
	var ov func(i, j int) bool
	ov = func(i, j int) bool {
		k := [2]int{i, j}
		if v, ok := memo[k]; ok {
			return v
		}
		var r bool
		switch {
		case i == len(sa) && j == len(sb):
			r = true
		case i < len(sa) && sa[i] == "**":
			r = ov(i+1, j) || j < len(sb) && ov(i, j+1)
		case j < len(sb) && sb[j] == "**":
			r = ov(i, j+1) || i < len(sa) && ov(i+1, j)
		case i == len(sa) || j == len(sb):
			r = false // the other side still needs a real segment
		default:
			r = segOverlap(ta[i], tb[j]) && ov(i+1, j+1)
		}
		memo[k] = r
		return r
	}
	return ov(0, 0)
}

// token is one element of a segment pattern.
type token struct {
	star   bool
	any    bool   // ?
	lit    rune   // literal when !star && !any && !class
	class  bool   // [...]
	neg    bool   // [^...]
	ranges []span // class members
}

type span struct{ lo, hi rune }

func tokenizeAll(segs []string) ([][]token, bool) {
	out := make([][]token, len(segs))
	for i, s := range segs {
		if s == "**" {
			continue
		}
		t, ok := tokenize(s)
		if !ok {
			return nil, false
		}
		out[i] = t
	}
	return out, true
}

func tokenize(s string) ([]token, bool) {
	var out []token
	for i := 0; i < len(s); {
		switch s[i] {
		case '*':
			out = append(out, token{star: true})
			i++
		case '?':
			out = append(out, token{any: true})
			i++
		case '[':
			i++
			t := token{class: true}
			if i < len(s) && s[i] == '^' {
				t.neg = true
				i++
			}
			closed := false
			for i < len(s) {
				if s[i] == ']' && len(t.ranges) > 0 {
					closed = true
					i++
					break
				}
				lo, n := utf8.DecodeRuneInString(s[i:])
				i += n
				hi := lo
				if i+1 < len(s) && s[i] == '-' && s[i+1] != ']' {
					i++
					hi, n = utf8.DecodeRuneInString(s[i:])
					i += n
				}
				if hi < lo {
					return nil, false
				}
				t.ranges = append(t.ranges, span{lo, hi})
			}
			if !closed {
				return nil, false
			}
			out = append(out, t)
		default:
			r, n := utf8.DecodeRuneInString(s[i:])
			out = append(out, token{lit: r})
			i += n
		}
	}
	return out, true
}

// segOverlap: can one path segment match both token lists?
func segOverlap(a, b []token) bool {
	memo := map[[2]int]bool{}
	var f func(i, j int) bool
	f = func(i, j int) bool {
		k := [2]int{i, j}
		if v, ok := memo[k]; ok {
			return v
		}
		var r bool
		switch {
		case i == len(a) && j == len(b):
			r = true
		case i < len(a) && a[i].star:
			// star takes nothing, or swallows b's next element (a star on b's
			// side is fine to swallow too: both can match the empty string).
			r = f(i+1, j) || j < len(b) && f(i, j+1)
		case j < len(b) && b[j].star:
			r = f(i, j+1) || i < len(a) && f(i+1, j)
		case i == len(a) || j == len(b):
			r = false
		default:
			r = charOverlap(a[i], b[j]) && f(i+1, j+1)
		}
		memo[k] = r
		return r
	}
	return f(0, 0)
}

// charOverlap: is there a character both single-character tokens accept
// (case-insensitively)?
func charOverlap(a, b token) bool {
	if a.any || b.any {
		return true
	}
	if !a.class && !b.class {
		return foldEq(a.lit, b.lit)
	}
	if !a.class {
		a, b = b, a // a is the class
	}
	if !b.class {
		for _, c := range folds(b.lit) {
			if inRanges(a.ranges, c) != a.neg {
				return true
			}
		}
		return false
	}
	switch {
	case a.neg && b.neg:
		return true // both exclude finitely many characters
	case !a.neg && !b.neg:
		if hasLetter(a.ranges) && hasLetter(b.ranges) {
			return true
		}
		for _, x := range a.ranges {
			for _, y := range b.ranges {
				if x.lo <= y.hi && y.lo <= x.hi {
					return true
				}
			}
		}
		return false
	default:
		pos, neg := a, b
		if pos.neg {
			pos, neg = b, a
		}
		// Overlap unless every member of pos is excluded by neg.
		return hasLetter(pos.ranges) || !covered(pos.ranges, neg.ranges)
	}
}

func foldEq(a, b rune) bool {
	for _, c := range folds(a) {
		if c == b {
			return true
		}
	}
	return false
}

// folds returns r and every rune it case-folds to.
func folds(r rune) []rune {
	out := []rune{r}
	for c := unicode.SimpleFold(r); c != r; c = unicode.SimpleFold(c) {
		out = append(out, c)
	}
	return out
}

func inRanges(rs []span, c rune) bool {
	for _, s := range rs {
		if s.lo <= c && c <= s.hi {
			return true
		}
	}
	return false
}

func hasLetter(rs []span) bool {
	for _, s := range rs {
		// Any range touching the ASCII letters, or reaching past ASCII, may
		// hold a cased letter.
		if s.hi > unicode.MaxASCII || s.lo <= 'z' && s.hi >= 'a' || s.lo <= 'Z' && s.hi >= 'A' {
			return true
		}
	}
	return false
}

// covered reports whether every rune in want lies inside have.
func covered(want, have []span) bool {
	h := append([]span(nil), have...)
	sort.Slice(h, func(i, j int) bool { return h[i].lo < h[j].lo })
	var merged []span
	for _, s := range h {
		if n := len(merged); n > 0 && s.lo <= merged[n-1].hi+1 {
			merged[n-1].hi = max(merged[n-1].hi, s.hi)
			continue
		}
		merged = append(merged, s)
	}
	for _, w := range want {
		ok := false
		for _, m := range merged {
			if m.lo <= w.lo && w.hi <= m.hi {
				ok = true
				break
			}
		}
		if !ok {
			return false
		}
	}
	return true
}

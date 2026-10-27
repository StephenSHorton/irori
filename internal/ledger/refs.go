package ledger

import (
	"regexp"
	"strconv"
	"strings"
)

var (
	fencedCode = regexp.MustCompile("```[\\s\\S]*?```")
	inlineCode = regexp.MustCompile("`[^`\\n]*`")

	// The three alternatives of tsukumo's pattern, anchored and tried in
	// order at each position, so a failed lookbehind retries at the next
	// position just as it does in JS.
	refURL  = regexp.MustCompile(`^github\.com/([\w.-]+)/([\w.-]+)/pull/(\d+)`)
	refName = regexp.MustCompile(`^([A-Za-z][\w.-]*(?:/[\w.-]+)?)#(\d{1,6})\b`)
	refBare = regexp.MustCompile(`^#(\d{2,5})\b`)

	targetURL  = regexp.MustCompile(`(?i)^(?:https?://)?(?:www\.)?github\.com/([\w.-]+)/([\w.-]+)/pull/(\d+)(?:[/?#].*)?$`)
	targetName = regexp.MustCompile(`^([\w.-]+)/([\w.-]+)#(\d+)$`)
)

func isWordByte(c byte) bool {
	return c == '_' || c >= '0' && c <= '9' || isLetter(c)
}

func isLetter(c byte) bool { return c >= 'A' && c <= 'Z' || c >= 'a' && c <= 'z' }

// prevBlocks reports whether the byte before p is an ASCII word byte or one
// of extra. JS \w is ASCII, so a non-ASCII rune never blocks; its last UTF-8
// byte is >= 0x80, so checking one byte is enough.
func prevBlocks(s string, p int, extra string) bool {
	if p == 0 {
		return false
	}
	c := s[p-1]
	return isWordByte(c) || strings.IndexByte(extra, c) >= 0
}

// prNumber reads a PR number; 0, overflow and junk are not PRs.
func prNumber(s string) (int, bool) {
	n, err := strconv.Atoi(s)
	return n, err == nil && n > 0
}

func extractRefs(text string, o RefOptions) []Ref {
	owner := o.Owner
	if owner == "" {
		owner, _, _ = strings.Cut(o.DefaultRepo, "/")
	}
	var out []Ref
	seen := map[string]bool{}
	add := func(repo, digits string) {
		n, ok := prNumber(digits)
		if !ok || repo == "" {
			return
		}
		k := key(repo, n)
		if seen[k] {
			return
		}
		seen[k] = true
		out = append(out, Ref{Repo: repo, Number: n})
	}

	// Code spans and blocks quote things; they don't name the subject.
	prose := fencedCode.ReplaceAllString(text, " ")
	prose = inlineCode.ReplaceAllString(prose, " ")

	for p := 0; p < len(prose); {
		rest, c := prose[p:], prose[p]
		if c == 'g' {
			if m := refURL.FindStringSubmatch(rest); m != nil {
				add(m[1]+"/"+m[2], m[3])
				p += len(m[0])
				continue
			}
		}
		if isLetter(c) && !prevBlocks(prose, p, "/<#-") {
			if m := refName.FindStringSubmatch(rest); m != nil {
				name, digits := m[1], m[2]
				lower := strings.ToLower(name)
				if lower == "pr" || lower == "pull" {
					// "PR#1407" / "pull#1407" mean the default repo.
					add(o.DefaultRepo, digits)
				} else {
					if a, ok := o.Aliases[lower]; ok {
						name = a
					}
					if strings.Contains(name, "/") {
						add(name, digits)
					} else if owner != "" {
						add(owner+"/"+name, digits)
					}
				}
				p += len(m[0])
				continue
			}
		}
		if c == '#' && !prevBlocks(prose, p, "/<#&@") {
			if m := refBare.FindStringSubmatch(rest); m != nil {
				add(o.DefaultRepo, m[1])
				p += len(m[0])
				continue
			}
		}
		p++
	}
	return out
}

func parseTarget(s string) (Ref, bool) {
	s = strings.TrimSpace(s)
	if m := targetURL.FindStringSubmatch(s); m != nil {
		if n, ok := prNumber(m[3]); ok {
			return Ref{Repo: m[1] + "/" + m[2], Number: n}, true
		}
		return Ref{}, false
	}
	if len(s) >= 3 && strings.EqualFold(s[:3], "pr:") {
		s = s[3:]
	}
	if m := targetName.FindStringSubmatch(s); m != nil {
		if n, ok := prNumber(m[3]); ok {
			return Ref{Repo: m[1] + "/" + m[2], Number: n}, true
		}
	}
	return Ref{}, false
}

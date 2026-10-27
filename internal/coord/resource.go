package coord

import (
	"fmt"
	"path"
	"regexp"
	"strconv"
	"strings"
)

// Resource grammar (design §6):
//
//	path:<repoKey>:<glob>   repoKey is "remote:<host>/<owner>/<name>" or
//	                        "root:<sha>" (internal/repo). The value after the
//	                        repoKey kind never contains ':' (repo strips ports),
//	                        so the repoKey ends at the third ':' and the glob is
//	                        everything after it (it may itself contain ':').
//	port:<n>                1..65535, no leading zeros.
//	<name>                  ^[a-z0-9][a-z0-9._-]*(/[a-z0-9._-]+)*$, e.g. gpu/qwen.
//
// Globs are POSIX, relative to the repo root: '/' separators, '*', '?',
// '[...]' and a whole-segment '**'. A leading "./" is stripped and a trailing
// '/' means the directory's contents ("src/" is "src/**"). Rejected: empty,
// any backslash, absolute paths ("/x", and "C:..." since a single letter then
// a colon reads as a drive), "." or ".." segments, empty
// segments ("a//b") and patterns path.Match calls malformed.

// Resource kinds.
const (
	ResourcePath  = "path"
	ResourcePort  = "port"
	ResourceNamed = "named"
)

// Resource is a parsed lease resource.
type Resource struct {
	Kind    string // ResourcePath, ResourcePort or ResourceNamed
	RepoKey string // path only
	Glob    string // path only, normalized
	Name    string // the canonical string (also for path and port)
}

var namedRE = regexp.MustCompile(`^[a-z0-9][a-z0-9._-]*(/[a-z0-9._-]+)*$`)

// ParseResource parses and normalizes a lease resource. Errors wrap
// ErrBadResource.
func ParseResource(s string) (Resource, error) {
	s = strings.TrimSpace(s)
	switch {
	case strings.HasPrefix(s, "path:"):
		rest := s[len("path:"):]
		kind, after, ok := strings.Cut(rest, ":")
		if !ok || (kind != "remote" && kind != "root") {
			return Resource{}, fmt.Errorf("%w: repoKey must start remote: or root: (%q)", ErrBadResource, s)
		}
		value, glob, ok := strings.Cut(after, ":")
		if !ok {
			return Resource{}, fmt.Errorf("%w: missing :<glob> after the repoKey (%q)", ErrBadResource, s)
		}
		repoKey := kind + ":" + value
		if err := checkRepoKey(repoKey); err != nil {
			return Resource{}, err
		}
		g, err := NormalizeGlob(glob)
		if err != nil {
			return Resource{}, err
		}
		return Resource{Kind: ResourcePath, RepoKey: repoKey, Glob: g, Name: "path:" + repoKey + ":" + g}, nil
	case strings.HasPrefix(s, "port:"):
		n := s[len("port:"):]
		p, err := strconv.Atoi(n)
		if err != nil || p < 1 || p > 65535 || strconv.Itoa(p) != n {
			return Resource{}, fmt.Errorf("%w: port must be 1-65535 (%q)", ErrBadResource, s)
		}
		return Resource{Kind: ResourcePort, Name: s}, nil
	case namedRE.MatchString(s):
		return Resource{Kind: ResourceNamed, Name: s}, nil
	}
	return Resource{}, fmt.Errorf("%w (%q)", ErrBadResource, s)
}

// NormalizeResource returns the canonical form of a lease resource.
func NormalizeResource(s string) (string, error) {
	r, err := ParseResource(s)
	return r.Name, err
}

// checkRepoKey accepts "remote:<value>" or "root:<value>" with a non-empty
// value free of ':', whitespace and backslashes.
func checkRepoKey(k string) error {
	kind, value, ok := strings.Cut(k, ":")
	if !ok || (kind != "remote" && kind != "root") || value == "" ||
		strings.ContainsAny(value, ": \t\r\n\\") {
		return fmt.Errorf("%w: bad repoKey %q", ErrBadResource, k)
	}
	return nil
}

// NormalizeGlob validates a repo-relative POSIX glob and returns its
// normalized form (see the grammar above). Errors wrap ErrBadResource.
func NormalizeGlob(g string) (string, error) {
	orig := g
	g = strings.TrimSpace(g)
	bad := func(why string) (string, error) {
		return "", fmt.Errorf("%w: glob %q %s", ErrBadResource, orig, why)
	}
	switch {
	case g == "":
		return bad("is empty")
	case strings.Contains(g, `\`):
		return bad("has a backslash; use /")
	case strings.HasPrefix(g, "/"), len(g) >= 2 && g[1] == ':' && isASCIILetter(g[0]):
		return bad("is absolute; use a path relative to the repo root")
	}
	for strings.HasPrefix(g, "./") {
		g = g[2:]
	}
	if g == "" {
		return bad("is empty")
	}
	if strings.HasSuffix(g, "/") {
		g += "**"
	}
	for _, seg := range strings.Split(g, "/") {
		switch seg {
		case "":
			return bad("has an empty segment")
		case ".", "..":
			return bad("has a . or .. segment")
		}
		if _, err := path.Match(seg, ""); err != nil {
			return bad("is malformed")
		}
	}
	return g, nil
}

func isASCIILetter(c byte) bool { return c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' }

// conflicts reports whether two parsed resources contend: the same canonical
// resource, or path globs that overlap within one repoKey.
func conflicts(a, b Resource) bool {
	if a.Kind == ResourcePath && b.Kind == ResourcePath {
		return a.RepoKey == b.RepoKey && overlapsNorm(a.Glob, b.Glob)
	}
	return a.Name == b.Name
}

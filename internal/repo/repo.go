// Package repo gives a working directory a repo_key that is the same on every
// device: "remote:<host>/<owner>/<name>" from the normalized origin remote,
// else "root:<sha>" from the repository's first root commit, else "" when the
// directory isn't in a git repo.
//
// Normalizing: git@github.com:Foo/Bar.git, https://github.com/Foo/Bar,
// ssh://git@github.com/foo/bar.git all become remote:github.com/foo/bar
// (lowercased, scheme/user/port/.git stripped).
//
// CONTRACT (implemented by the repo work package; keep these signatures):
package repo

// Resolver caches keys per directory. Safe for concurrent use.
type Resolver struct {
	impl *resolver
}

// New makes a resolver. git runs with a short timeout; failures yield "".
func New() *Resolver { return newResolver() }

// Key returns the repo_key for dir (any directory inside a repo works).
func (r *Resolver) Key(dir string) string { return r.key(dir) }

// NormalizeRemote turns a remote URL into "host/owner/name", or "" if it
// can't be parsed.
func NormalizeRemote(url string) string { return normalizeRemote(url) }

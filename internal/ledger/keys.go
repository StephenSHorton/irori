package ledger

import (
	"strconv"
	"strings"
)

const githubPrefix = "remote:github.com/"

func key(repo string, n int) string {
	return "pr:" + strings.ToLower(strings.TrimSpace(repo)) + "#" + strconv.Itoa(n)
}

func repoKey(repo string) string {
	return githubPrefix + strings.ToLower(strings.TrimSpace(repo))
}

// repoFromKey returns the "owner/name" part of a remote:github.com/ key, or
// "" when the key is another kind or the rest is not exactly owner/name.
func repoFromKey(k string) string {
	rest, ok := strings.CutPrefix(k, githubPrefix)
	if !ok {
		return ""
	}
	owner, name, ok := strings.Cut(rest, "/")
	if !ok || owner == "" || name == "" || strings.Contains(name, "/") {
		return ""
	}
	return rest
}

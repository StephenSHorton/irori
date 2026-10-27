package capture

import (
	"path/filepath"
	"regexp"
	"strings"
)

// gitDirFlag finds `git -C <dir>` (the dir bare, or in single or double
// quotes); cdPrefix finds a leading `cd <dir> &&` / `cd <dir>;` (also
// Set-Location, Push-Location).
var (
	gitDirFlag = regexp.MustCompile(`\bgit(?:\.exe)?\s+(?:-c\s+\S+\s+)*-C\s+("[^"]+"|'[^']+'|\S+)`)
	cdPrefix   = regexp.MustCompile(`(?i)^\s*(?:cd|set-location|push-location|sl)\s+(?:-path\s+)?("[^"]+"|'[^']+'|[^\s;&|]+)\s*(?:&&|;)`)
)

// commandDir is the directory a commit command really ran git in: the
// `git -C <dir>` it names, else the dir of a leading cd, else cwd. Agents
// here often commit from their home folder with `git -C <repo> commit`, so
// matching only on the shell's cwd misses the committer.
func commandDir(command, cwd string) string {
	m := gitDirFlag.FindStringSubmatch(command)
	if m == nil {
		m = cdPrefix.FindStringSubmatch(command)
	}
	if m == nil {
		return cwd
	}
	dir := strings.Trim(m[1], `"'`)
	if dir == "" {
		return cwd
	}
	if isAbsAnyOS(dir) || cwd == "" {
		return dir
	}
	return filepath.Join(cwd, filepath.FromSlash(dir))
}

// isAbsAnyOS accepts Windows drive paths (either slash) and POSIX paths.
func isAbsAnyOS(p string) bool {
	if filepath.IsAbs(p) || strings.HasPrefix(p, "/") || strings.HasPrefix(p, `\\`) {
		return true
	}
	return len(p) >= 3 && p[1] == ':' && (p[2] == '\\' || p[2] == '/')
}

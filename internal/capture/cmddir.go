package capture

import (
	"os"
	"path/filepath"
	"strings"
)

// commitSubcommands are the git subcommands that can make a commit.
var commitSubcommands = map[string]bool{
	"commit": true, "merge": true, "cherry-pick": true, "revert": true,
	"rebase": true, "am": true, "pull": true,
}

// commandDir is the directory a commit command really ran git in. It walks
// the command's segments (split on &&, ||, ;, | and newlines, outside
// quotes), following cd / Set-Location / pushd, and for the first git
// segment whose subcommand can commit it applies that git's own -C options.
// Text inside a commit message is never read as a -C. Agents here often
// commit from their home folder with `git -C <repo> commit` or
// `cd ~/projects/x && git commit`, so matching only on the shell's cwd
// misses the committer. When nothing commits it returns the directory the
// segments ended in (cwd when there is no cd).
func commandDir(command, cwd string) string {
	cur := cwd
	for _, seg := range splitSegments(command) {
		w := shellWords(seg)
		if len(w) == 0 {
			continue
		}
		switch prog := strings.ToLower(progBase(w[0])); prog {
		case "cd", "set-location", "sl", "pushd", "push-location", "chdir":
			args := w[1:]
			if len(args) > 0 && strings.EqualFold(args[0], "-path") || len(args) > 0 && strings.EqualFold(args[0], "-literalpath") {
				args = args[1:]
			}
			if len(args) > 0 && args[0] != "-" {
				cur = resolveDir(args[0], cur)
			}
		case "git":
			dir := cur
			i := 1
			for i < len(w) && strings.HasPrefix(w[i], "-") {
				switch {
				case w[i] == "-C" && i+1 < len(w):
					dir = resolveDir(w[i+1], dir)
					i += 2
				case w[i] == "-c" && i+1 < len(w):
					i += 2
				case (w[i] == "--git-dir" || w[i] == "--work-tree" || w[i] == "--namespace") && i+1 < len(w):
					i += 2
				default:
					i++
				}
			}
			if i < len(w) && commitSubcommands[w[i]] {
				return dir
			}
		}
	}
	return cur
}

// progBase is a program word without its directory and .exe.
func progBase(w string) string {
	w = filepath.Base(filepath.FromSlash(strings.ReplaceAll(w, `\`, "/")))
	return strings.TrimSuffix(strings.TrimSuffix(w, ".exe"), ".EXE")
}

// resolveDir expands ~, $HOME, $env:USERPROFILE and %USERPROFILE% at the
// start of p and joins a relative result to cur.
func resolveDir(p, cur string) string {
	if home, err := os.UserHomeDir(); err == nil {
		for _, pre := range []string{"~", "$HOME", "${HOME}", "$env:USERPROFILE", "$env:HOME", "%USERPROFILE%"} {
			if p == pre || strings.HasPrefix(p, pre+"/") || strings.HasPrefix(p, pre+`\`) {
				p = home + p[len(pre):]
				break
			}
		}
	}
	p = gitBashPath(p)
	if isAbsAnyOS(p) || cur == "" {
		return p
	}
	return filepath.Join(cur, filepath.FromSlash(p))
}

// gitBashPath turns Git Bash's /c/Users/... into C:/Users/... .
func gitBashPath(p string) string {
	if len(p) >= 3 && p[0] == '/' && p[2] == '/' && (p[1] >= 'a' && p[1] <= 'z' || p[1] >= 'A' && p[1] <= 'Z') {
		return strings.ToUpper(p[1:2]) + ":" + p[2:]
	}
	if len(p) == 2 && p[0] == '/' && (p[1] >= 'a' && p[1] <= 'z' || p[1] >= 'A' && p[1] <= 'Z') {
		return strings.ToUpper(p[1:2]) + ":/"
	}
	return p
}

// splitSegments splits a command line on &&, ||, ;, | and newlines that are
// outside single or double quotes.
func splitSegments(s string) []string {
	var out []string
	var b strings.Builder
	var q rune
	rs := []rune(s)
	for i := 0; i < len(rs); i++ {
		r := rs[i]
		switch {
		case q != 0:
			if r == q {
				q = 0
			}
			b.WriteRune(r)
		case r == '\'' || r == '"':
			q = r
			b.WriteRune(r)
		case r == ';' || r == '\n' || r == '|' || r == '&' && i+1 < len(rs) && rs[i+1] == '&':
			if (r == '&' || r == '|') && i+1 < len(rs) && rs[i+1] == r {
				i++
			}
			out = append(out, b.String())
			b.Reset()
		default:
			b.WriteRune(r)
		}
	}
	return append(out, b.String())
}

// shellWords splits one segment into words: quotes group (and are removed),
// a backslash before a space escapes it (Git Bash), other backslashes are
// kept (Windows paths).
func shellWords(s string) []string {
	var out []string
	var b strings.Builder
	in, q := false, rune(0)
	rs := []rune(s)
	for i := 0; i < len(rs); i++ {
		r := rs[i]
		switch {
		case q != 0:
			if r == q {
				q = 0
			} else {
				b.WriteRune(r)
			}
		case r == '\'' || r == '"':
			q, in = r, true
		case r == '\\' && i+1 < len(rs) && rs[i+1] == ' ':
			b.WriteRune(' ')
			in = true
			i++
		case r == ' ' || r == '\t' || r == '\r':
			if in {
				out = append(out, b.String())
				b.Reset()
				in = false
			}
		default:
			b.WriteRune(r)
			in = true
		}
	}
	if in {
		out = append(out, b.String())
	}
	return out
}

// isAbsAnyOS accepts Windows drive paths (either slash) and POSIX paths.
func isAbsAnyOS(p string) bool {
	if filepath.IsAbs(p) || strings.HasPrefix(p, "/") || strings.HasPrefix(p, `\\`) {
		return true
	}
	return len(p) >= 3 && p[1] == ':' && (p[2] == '\\' || p[2] == '/')
}

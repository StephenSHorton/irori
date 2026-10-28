package main

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

// A Mac app opened from Finder, the Dock or a login item gets launchd's
// PATH (/usr/bin:/bin:/usr/sbin:/sbin), not the one the user's shell sets
// up. The desktop app starts `claude` (roster's resume) and git, and the
// agents it starts inherit its environment, so they would miss Homebrew,
// ~/.local/bin and every version manager. fixGUIPath asks the login shell
// for its PATH once at startup, the way editors do, and puts it in front.

// pathMarker brackets the PATH in the shell's output, so whatever a login
// shell prints on start (a motd, a banner) is skipped.
const pathMarker = "__IRORI_PATH__"

// fallbackPath is added when the shell can't be asked: where the Claude Code
// installer, Homebrew on Apple silicon and Homebrew on Intel put commands.
func fallbackPath() []string {
	dirs := []string{"/opt/homebrew/bin", "/usr/local/bin"}
	if home, err := os.UserHomeDir(); err == nil {
		dirs = append([]string{filepath.Join(home, ".local", "bin")}, dirs...)
	}
	return dirs
}

func fixGUIPath() {
	cur := filepath.SplitList(os.Getenv("PATH"))
	shellPath := loginShellPath()
	add := filepath.SplitList(shellPath)
	if shellPath == "" {
		add = fallbackPath()
	}
	_ = os.Setenv("PATH", strings.Join(mergePath(add, cur), string(os.PathListSeparator)))
}

// loginShellPath is $PATH as the user's login shell sets it, or "".
func loginShellPath() string {
	shell := os.Getenv("SHELL")
	if shell == "" || !filepath.IsAbs(shell) {
		shell = "/bin/zsh"
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, shell, "-ilc", `printf '`+pathMarker+`%s`+pathMarker+`' "$PATH"`)
	// A shell plugin that forks something holding stdout open must not
	// stall startup past the timeout.
	cmd.WaitDelay = time.Second
	out, err := cmd.Output()
	if err != nil {
		return ""
	}
	return between(string(out), pathMarker)
}

// between returns the text between the first two markers in s, or "".
func between(s, marker string) string {
	_, rest, ok := strings.Cut(s, marker)
	if !ok {
		return ""
	}
	v, _, ok := strings.Cut(rest, marker)
	if !ok {
		return ""
	}
	return strings.TrimSpace(v)
}

// mergePath is first followed by the entries of then it lacks, without
// empty entries or repeats.
func mergePath(first, then []string) []string {
	seen := map[string]bool{}
	var out []string
	for _, list := range [][]string{first, then} {
		for _, d := range list {
			if d == "" || seen[d] {
				continue
			}
			seen[d] = true
			out = append(out, d)
		}
	}
	return out
}

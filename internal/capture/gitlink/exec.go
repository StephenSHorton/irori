package gitlink

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"
)

type execGit struct{}

// Run runs `git -C dir args...` (no -C when dir is "") from PATH with no
// shell and no console window. It returns stdout untouched, so -z output
// keeps its NULs. A failed run's error carries the first line of stderr.
func (execGit) Run(ctx context.Context, dir string, args ...string) (string, error) {
	full := make([]string, 0, len(args)+3)
	full = append(full, "--no-pager")
	if dir != "" {
		full = append(full, "-C", dir)
	}
	full = append(full, args...)
	cmd := exec.CommandContext(ctx, "git", full...)
	cmd.Env = append(gitEnv(os.Environ()),
		"GIT_TERMINAL_PROMPT=0", "GIT_OPTIONAL_LOCKS=0", "GIT_PAGER=cat", "PAGER=cat")
	cmd.WaitDelay = 500 * time.Millisecond
	hideWindow(cmd)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		if ctx.Err() != nil {
			return "", ctx.Err()
		}
		name := ""
		if len(args) > 0 {
			name = args[0]
		}
		msg, _, _ := strings.Cut(strings.TrimSpace(stderr.String()), "\n")
		if msg = strings.TrimSpace(msg); msg != "" {
			return "", fmt.Errorf("git %s: %w: %s", name, err, msg)
		}
		return "", fmt.Errorf("git %s: %w", name, err)
	}
	return stdout.String(), nil
}

// repoEnvVars point git at one repository regardless of -C. Inherited from a
// hook or another tool, they would send every run to that repo.
var repoEnvVars = []string{
	"GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_INDEX_FILE",
	"GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES",
	"GIT_CEILING_DIRECTORIES", "GIT_DISCOVERY_ACROSS_FILESYSTEM", "GIT_NAMESPACE",
}

// gitEnv returns env without the repo-selecting git variables.
func gitEnv(env []string) []string {
	out := make([]string, 0, len(env))
	for _, kv := range env {
		name, _, _ := strings.Cut(kv, "=")
		drop := false
		for _, v := range repoEnvVars {
			if sameEnvName(name, v) {
				drop = true
				break
			}
		}
		if !drop {
			out = append(out, kv)
		}
	}
	return out
}

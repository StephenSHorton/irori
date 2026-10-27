package gitlink

import (
	"context"
	"fmt"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"
)

// maxSubject caps Commit.Subject, in runes.
const maxSubject = 200

func describe(ctx context.Context, g Git, root, sha string) (Commit, error) {
	if !isSHA(sha) {
		return Commit{}, fmt.Errorf("gitlink: %q is not a full object id", sha)
	}
	out, err := g.Run(ctx, root, "-c", "log.showSignature=false", "show", "-s", "--no-color",
		"--format=%H%x00%P%x00%ct%x00%an%x00%s", sha, "--")
	if err != nil {
		return Commit{}, err
	}
	parts := strings.SplitN(strings.TrimRight(out, "\r\n"), "\x00", 5)
	if len(parts) != 5 || !isSHA(parts[0]) {
		return Commit{}, fmt.Errorf("gitlink: unexpected git show output for %s", sha)
	}
	ct, err := strconv.ParseInt(parts[2], 10, 64)
	if err != nil {
		return Commit{}, fmt.Errorf("gitlink: bad committer time %q for %s", parts[2], sha)
	}
	c := Commit{
		SHA:     strings.ToLower(parts[0]),
		Parents: strings.Fields(strings.ToLower(parts[1])),
		At:      time.Unix(ct, 0),
		Author:  parts[3],
		Subject: truncRunes(parts[4], maxSubject),
	}

	// The contract's `-m --first-parent` still prints one diff per parent
	// in diff-tree (git 2.55), and --no-commit-id hides where each starts.
	// --diff-merges=first-parent (git 2.31+) is the form that keeps only
	// the first parent's diff, which is what §5 means.
	args := []string{"diff-tree", "-r", "--no-commit-id", "--no-renames", "--raw", "--no-abbrev",
		"--diff-merges=first-parent", "-z"}
	if len(c.Parents) == 0 {
		args = append(args, "--root")
	}
	out, err = g.Run(ctx, root, append(args, c.SHA, "--")...)
	if err != nil {
		return Commit{}, err
	}
	files, err := parseRawZ(out)
	if err != nil {
		return Commit{}, fmt.Errorf("gitlink: diff-tree %s: %w", sha, err)
	}
	if len(files) > MaxFiles {
		c.FilesOmitted = len(files) - MaxFiles
		files = files[:MaxFiles]
	}
	c.Files = files
	return c, nil
}

// parseRawZ reads `diff-tree --raw -z` output: for each path a
// ":<mode> <mode> <old> <new> <status>" field then the path, each ending in
// NUL. --no-renames means one path per entry.
func parseRawZ(out string) ([]FileChange, error) {
	files := []FileChange{}
	fields := strings.Split(out, "\x00")
	for i := 0; i < len(fields); i++ {
		meta := strings.TrimLeft(fields[i], "\r\n")
		if meta == "" {
			continue
		}
		if meta[0] != ':' {
			return nil, fmt.Errorf("unexpected field %q", meta)
		}
		m := strings.Fields(meta[1:])
		if len(m) != 5 || m[4] == "" {
			return nil, fmt.Errorf("unexpected field %q", meta)
		}
		if i+1 >= len(fields) {
			return nil, fmt.Errorf("entry %q has no path", meta)
		}
		i++
		files = append(files, FileChange{
			Path:   fields[i],
			Blob:   strings.ToLower(m[3]),
			Status: m[4][:1],
		})
	}
	return files, nil
}

func truncRunes(s string, n int) string {
	if utf8.RuneCountInString(s) <= n {
		return s
	}
	i := 0
	for k := 0; k < n; k++ {
		_, size := utf8.DecodeRuneInString(s[i:])
		i += size
	}
	return s[:i]
}

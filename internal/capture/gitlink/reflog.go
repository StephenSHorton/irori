package gitlink

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

func gitDir(root string) (string, error) {
	dot := filepath.Join(root, ".git")
	fi, err := os.Stat(dot)
	if err != nil {
		return "", err
	}
	if fi.IsDir() {
		return dot, nil
	}
	data, err := os.ReadFile(dot)
	if err != nil {
		return "", err
	}
	data = bytes.TrimPrefix(data, []byte("\xef\xbb\xbf"))
	line, _, _ := strings.Cut(string(data), "\n")
	line = strings.TrimSpace(line)
	p, ok := strings.CutPrefix(line, "gitdir:")
	if !ok {
		return "", fmt.Errorf("gitlink: %s has no gitdir line", dot)
	}
	p = strings.TrimSpace(p)
	if p == "" {
		return "", fmt.Errorf("gitlink: %s has an empty gitdir", dot)
	}
	p = filepath.FromSlash(p)
	if !filepath.IsAbs(p) {
		p = filepath.Join(root, p)
	}
	return filepath.Clean(p), nil
}

func parseReflog(data []byte) ([]ReflogEntry, int) {
	end := bytes.LastIndexByte(data, '\n')
	if end < 0 {
		return nil, 0
	}
	var out []ReflogEntry
	for _, line := range strings.Split(string(data[:end]), "\n") {
		if e, ok := parseReflogLine(strings.TrimSuffix(line, "\r")); ok {
			out = append(out, e)
		}
	}
	return out, end + 1
}

// parseReflogLine parses "<old> <new> <name> <<email>> <unix> <tz>\t<msg>".
// Git writes no tab when the message is empty.
func parseReflogLine(line string) (ReflogEntry, bool) {
	head, msg, _ := strings.Cut(line, "\t")
	old, rest, ok := strings.Cut(head, " ")
	if !ok || !isSHA(old) {
		return ReflogEntry{}, false
	}
	nw, rest, ok := strings.Cut(rest, " ")
	if !ok || !isSHA(nw) || len(nw) != len(old) {
		return ReflogEntry{}, false
	}
	lt := strings.IndexByte(rest, '<')
	gt := strings.LastIndexByte(rest, '>')
	if lt < 0 || gt < lt {
		return ReflogEntry{}, false
	}
	name := strings.TrimSpace(rest[:lt])
	email := rest[lt+1 : gt]
	fields := strings.Fields(rest[gt+1:])
	if len(fields) != 2 {
		return ReflogEntry{}, false
	}
	unix, err := strconv.ParseInt(fields[0], 10, 64)
	if err != nil {
		return ReflogEntry{}, false
	}
	loc, ok := parseTZ(fields[1])
	if !ok {
		return ReflogEntry{}, false
	}
	return ReflogEntry{
		Old:     strings.ToLower(old),
		New:     strings.ToLower(nw),
		Name:    name,
		Email:   email,
		At:      time.Unix(unix, 0).In(loc),
		Message: msg,
	}, true
}

// parseTZ reads git's "+hhmm" / "-hhmm" offset.
func parseTZ(s string) (*time.Location, bool) {
	if len(s) != 5 || (s[0] != '+' && s[0] != '-') {
		return nil, false
	}
	hh, err1 := strconv.Atoi(s[1:3])
	mm, err2 := strconv.Atoi(s[3:5])
	if err1 != nil || err2 != nil || mm > 59 {
		return nil, false
	}
	off := hh*3600 + mm*60
	if s[0] == '-' {
		off = -off
	}
	return time.FixedZone(s, off), true
}

// isSHA reports a full SHA-1 (40) or SHA-256 (64) hex object id.
func isSHA(s string) bool {
	if len(s) != 40 && len(s) != 64 {
		return false
	}
	for i := 0; i < len(s); i++ {
		c := s[i]
		if !('0' <= c && c <= '9' || 'a' <= c && c <= 'f' || 'A' <= c && c <= 'F') {
			return false
		}
	}
	return true
}

func zeroSHA(s string) bool {
	if s == "" {
		return false
	}
	for i := 0; i < len(s); i++ {
		if s[i] != '0' {
			return false
		}
	}
	return true
}

// newCommits walks <old>..<new>. When <old> is gone (pruned, or an object
// git can no longer read) it walks <new> alone, which the seen filter and
// MaxWalk keep bounded. Commits git lists that seen holds are dropped;
// skipped counts every commit past MaxWalk in the range, seen or not (the
// walk keeps the newest MaxWalk, so the skipped ones are the oldest).
func newCommits(ctx context.Context, g Git, root string, e ReflogEntry, seen func(string) bool) ([]string, int, error) {
	if seen == nil {
		seen = func(string) bool { return false }
	}
	if !isSHA(e.New) || (e.Old != "" && !zeroSHA(e.Old) && !isSHA(e.Old)) {
		return nil, 0, errors.New("gitlink: reflog entry has a malformed object id")
	}
	if zeroSHA(e.New) || seen(e.New) {
		return nil, 0, nil
	}
	ranges := []string{e.New}
	if e.Old != "" && !zeroSHA(e.Old) {
		ranges = []string{e.Old + ".." + e.New, e.New}
	}
	var out string
	var err error
	var rng string
	for _, rng = range ranges {
		out, err = g.Run(ctx, root, "rev-list", "--reverse", "--max-count="+strconv.Itoa(MaxWalk), rng, "--")
		if err == nil || ctx.Err() != nil {
			break
		}
	}
	if err != nil {
		return nil, 0, err
	}
	listed := strings.Fields(out)
	skipped := 0
	if len(listed) >= MaxWalk {
		n, err := g.Run(ctx, root, "rev-list", "--count", rng, "--")
		if err != nil {
			return nil, 0, err
		}
		total, err := strconv.Atoi(strings.TrimSpace(n))
		if err != nil {
			return nil, 0, fmt.Errorf("gitlink: rev-list --count: %q", strings.TrimSpace(n))
		}
		if total > len(listed) {
			skipped = total - len(listed)
		}
	}
	var shas []string
	for _, s := range listed {
		s = strings.ToLower(s)
		if !seen(s) {
			shas = append(shas, s)
		}
	}
	return shas, skipped, nil
}

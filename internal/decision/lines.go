package decision

import (
	"crypto/sha256"
	"encoding/hex"
	"io"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"
)

func validate(d *Decision) error {
	d.Statement = strings.TrimSpace(d.Statement)
	d.Why = strings.TrimSpace(d.Why)
	if d.Statement == "" {
		return ErrEmpty
	}
	if utf8.RuneCountInString(d.Statement) > MaxStatement || utf8.RuneCountInString(d.Why) > MaxWhy {
		return ErrTooLong
	}
	return nil
}

func parseLines(s string) (int, int, error) {
	s = strings.TrimSpace(s)
	if s == "" {
		return 0, 0, nil
	}
	a, b, ranged := strings.Cut(s, "-")
	start, ok := lineNumber(a)
	if !ok {
		return 0, 0, ErrBadLines
	}
	end := start
	if ranged {
		if end, ok = lineNumber(b); !ok {
			return 0, 0, ErrBadLines
		}
	}
	if end < start {
		return 0, 0, ErrBadLines
	}
	return start, end, nil
}

// lineNumber reads a 1-based line number made only of digits.
func lineNumber(s string) (int, bool) {
	s = strings.TrimSpace(s)
	if s == "" {
		return 0, false
	}
	for _, r := range s {
		if r < '0' || r > '9' {
			return 0, false
		}
	}
	n, err := strconv.Atoi(s)
	if err != nil || n < 1 {
		return 0, false
	}
	return n, true
}

// splitLines splits file content into normalised lines. A final newline does
// not start another line, so empty content has none.
func splitLines(data []byte) []string {
	if len(data) == 0 {
		return nil
	}
	s := strings.TrimSuffix(string(data), "\n")
	lines := strings.Split(s, "\n")
	for i, l := range lines {
		lines[i] = normLine(l)
	}
	return lines
}

// normLine drops every "\r" and any trailing whitespace.
func normLine(l string) string {
	return strings.TrimRightFunc(strings.ReplaceAll(l, "\r", ""), unicode.IsSpace)
}

// hashLines is the line-anchor hash of already normalised lines.
func hashLines(lines []string) string {
	h := sha256.New()
	for i, l := range lines {
		if i > 0 {
			io.WriteString(h, "\n")
		}
		io.WriteString(h, l)
	}
	return hex.EncodeToString(h.Sum(nil))
}

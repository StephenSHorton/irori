package presence

import (
	"encoding/json"
	"io"
	"strconv"
	"strings"
	"time"
)

// maxSessionFile caps how much of a session json we read; real ones are <1 KB.
const maxSessionFile = 64 << 10

// session is the part of a <pid>.json file the roster uses.
type session struct {
	PID          int
	ID           string
	Cwd          string
	Name         string
	NameSource   string // who set Name: "user" for /rename; other values are Claude Code's
	Status       string
	Kind         string
	Entrypoint   string
	Version      string
	PeerFeatures []string
	// ProcStart is when the session's process started, in the unit Claude
	// Code wrote it: FILETIME ticks on Windows; on macOS and Linux Claude
	// writes `ps -o lstart=` text (UTC), which readSession turns into Unix
	// seconds. 0 when absent or unreadable.
	ProcStart uint64
	StartedAt time.Time
	UpdatedAt time.Time
}

// isSessionFile reports whether name is <digits>.json. The <pid>.<hash>.key
// files next to them are secrets and must never match.
func isSessionFile(name string) bool {
	stem, ok := strings.CutSuffix(name, ".json")
	if !ok || stem == "" {
		return false
	}
	for _, r := range stem {
		if r < '0' || r > '9' {
			return false
		}
	}
	return true
}

// readSession parses one session file. ok is false when it has no usable pid
// or session id.
func readSession(r io.Reader) (session, bool) {
	raw, err := io.ReadAll(io.LimitReader(r, maxSessionFile))
	if err != nil {
		return session{}, false
	}
	var m map[string]any
	// Invalid JSON is unreadable. Don't try to repair it: in a Windows path
	// "\t" or "\n" is path text, so any guess would corrupt cwd.
	if err := json.Unmarshal(raw, &m); err != nil {
		return session{}, false
	}
	s := session{
		PID:          int(num(m["pid"])),
		ID:           str(m["sessionId"]),
		Cwd:          str(m["cwd"]),
		Name:         str(m["name"]),
		NameSource:   str(m["nameSource"]),
		Status:       str(m["status"]),
		Kind:         str(m["kind"]),
		Entrypoint:   str(m["entrypoint"]),
		Version:      str(m["version"]),
		PeerFeatures: strs(m["peerFeatures"]),
		StartedAt:    unixMilli(num(m["startedAt"])),
	}
	switch v := m["procStart"].(type) {
	case string:
		if n, err := strconv.ParseUint(v, 10, 64); err == nil {
			s.ProcStart = n
		} else if t, ok := parseLstart(v); ok {
			s.ProcStart = uint64(t.Unix())
		}
	case float64:
		if v > 0 {
			s.ProcStart = uint64(v)
		}
	}
	updated := max(num(m["updatedAt"]), num(m["statusUpdatedAt"]))
	s.UpdatedAt = unixMilli(updated)
	if s.PID <= 0 || s.ID == "" {
		return session{}, false
	}
	return s, true
}

func str(v any) string {
	s, _ := v.(string)
	return s
}

func num(v any) float64 {
	switch n := v.(type) {
	case float64:
		return n
	case string:
		f, _ := strconv.ParseFloat(n, 64)
		return f
	}
	return 0
}

func strs(v any) []string {
	list, _ := v.([]any)
	var out []string
	for _, x := range list {
		if s, ok := x.(string); ok {
			out = append(out, s)
		}
	}
	return out
}

func unixMilli(ms float64) time.Time {
	if ms <= 0 {
		return time.Time{}
	}
	return time.UnixMilli(int64(ms))
}

// lstartLayout is `ps -o lstart=` under LC_ALL=C, which is how Claude Code
// records procStart off Windows (with TZ=UTC): "Mon Oct 27 14:29:03 2026".
const lstartLayout = "Mon Jan _2 15:04:05 2006"

// parseLstart reads an lstart timestamp as UTC.
func parseLstart(v string) (time.Time, bool) {
	// Collapsing runs of spaces turns "Oct  7" into "Oct 7", which _2 reads.
	t, err := time.ParseInLocation(lstartLayout, strings.Join(strings.Fields(v), " "), time.UTC)
	if err != nil || t.Year() < 2000 {
		return time.Time{}, false
	}
	return t, true
}

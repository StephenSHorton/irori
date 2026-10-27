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
	Status       string
	Kind         string
	Entrypoint   string
	Version      string
	PeerFeatures []string
	ProcStart    uint64 // FILETIME ticks on Windows; 0 when absent or unreadable
	StartedAt    time.Time
	UpdatedAt    time.Time
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
		Status:       str(m["status"]),
		Kind:         str(m["kind"]),
		Entrypoint:   str(m["entrypoint"]),
		Version:      str(m["version"]),
		PeerFeatures: strs(m["peerFeatures"]),
		StartedAt:    unixMilli(num(m["startedAt"])),
	}
	switch v := m["procStart"].(type) {
	case string:
		s.ProcStart, _ = strconv.ParseUint(v, 10, 64)
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

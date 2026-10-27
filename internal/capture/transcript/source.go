package transcript

import (
	"bytes"
	"path"
	"strings"
)

// sourceFor matches path against projects/<slug>/<sid>.jsonl and
// projects/<slug>/<sid>/subagents/**/agent-<id>.jsonl. Both slash styles
// are accepted; the projects prefix compares case-insensitively when it
// looks like a Windows path.
func sourceFor(projectsDir, p string) (Source, bool) {
	base := strings.TrimRight(cleanSlash(projectsDir), "/")
	full := cleanSlash(p)
	if base == "" || len(full) <= len(base)+1 || full[len(base)] != '/' {
		return Source{}, false
	}
	if !samePrefix(full[:len(base)], base) {
		return Source{}, false
	}
	parts := strings.Split(full[len(base)+1:], "/")
	for _, s := range parts {
		if s == "" || s == "." || s == ".." {
			return Source{}, false
		}
	}
	switch {
	case len(parts) == 2:
		sid, ok := strings.CutSuffix(parts[1], ".jsonl")
		if !ok || !validID(sid) {
			return Source{}, false
		}
		return Source{SessionID: sid}, true
	case len(parts) >= 4 && parts[2] == "subagents":
		sid := parts[1]
		name := parts[len(parts)-1]
		id, ok := strings.CutPrefix(name, "agent-")
		if !ok {
			return Source{}, false
		}
		id, ok = strings.CutSuffix(id, ".jsonl")
		if !ok || !validID(sid) || !validID(id) {
			return Source{}, false
		}
		return Source{SessionID: sid, AgentID: id}, true
	}
	return Source{}, false
}

// validID reports whether s matches [A-Za-z0-9_-]+.
func validID(s string) bool {
	if s == "" {
		return false
	}
	for i := 0; i < len(s); i++ {
		c := s[i]
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '_' || c == '-') {
			return false
		}
	}
	return true
}

// cleanSlash turns backslashes into slashes and cleans the result.
func cleanSlash(p string) string {
	if p == "" {
		return ""
	}
	return path.Clean(strings.ReplaceAll(p, `\`, "/"))
}

// samePrefix compares two slash paths, ignoring case for Windows-looking ones.
func samePrefix(a, b string) bool {
	if windowsish(a) || windowsish(b) {
		return strings.EqualFold(a, b)
	}
	return a == b
}

// windowsish reports a drive-letter path ("C:/..." or "C:\...") or a UNC path.
func windowsish(p string) bool {
	if len(p) >= 2 && p[1] == ':' && (p[0] >= 'a' && p[0] <= 'z' || p[0] >= 'A' && p[0] <= 'Z') {
		return true
	}
	return strings.HasPrefix(p, `\\`) || strings.HasPrefix(p, "//")
}

var apiRequestPrefix = []byte(`{"type":"api-request`)

// skipLine reports blank lines and api-request rows.
func skipLine(line []byte) bool {
	t := bytes.TrimLeft(line, " \t")
	if len(bytes.TrimSpace(t)) == 0 {
		return true
	}
	return bytes.HasPrefix(t, apiRequestPrefix)
}

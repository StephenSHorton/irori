package mcpserver

import (
	"crypto/rand"
	"encoding/hex"
	"strings"
)

// Session id precedence for irori_join (suzuri.md §3): the call's _meta, then
// the environment, then a random id fixed for the process. The Core gives the
// same session id the same memberKey, so a restarted Core or a reconnect
// rebinds the same member. A sessionless process is its own member: the key
// is never derived from the join name, which anyone can pick, so a new
// process under a reused name can't take over another member's cards or
// inbox. A script that posts a card and later updates it from a new process
// keeps the right to by setting IRORI_SESSION_ID to an id of its own (a
// random token it keeps, not a name).
var (
	sessionMetaKeys = []string{"session_id", "grok_session_id", "conversation_id", "mcp_session_id"}
	sessionEnvKeys  = []string{"IRORI_SESSION_ID", "GROK_SESSION_ID", "XAI_SESSION_ID"}
)

// maxSessionLen bounds a client-supplied session id.
const maxSessionLen = 200

func sessionFromMeta(meta map[string]any) string {
	for _, k := range sessionMetaKeys {
		if v, ok := meta[k].(string); ok {
			if v = cleanSession(v); v != "" {
				return v
			}
		}
	}
	return ""
}

func sessionFromEnv(getenv func(string) string) string {
	if getenv == nil {
		return ""
	}
	for _, k := range sessionEnvKeys {
		if v := cleanSession(getenv(k)); v != "" {
			return v
		}
	}
	return ""
}

func cleanSession(s string) string {
	s = oneLine(s, maxSessionLen)
	if strings.ContainsAny(s, " ") {
		s = strings.ReplaceAll(s, " ", "_")
	}
	return s
}

func randomSession() string {
	b := make([]byte, 12)
	_, _ = rand.Read(b)
	return "sess_" + hex.EncodeToString(b)
}

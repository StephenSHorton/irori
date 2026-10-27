package mcpserver

import (
	"crypto/rand"
	"encoding/hex"
	"strings"
)

// Session id precedence for irori_join (suzuri.md §3): the call's _meta, then
// the environment, then one random id per process. The Core gives the same
// session id the same memberKey, so a restarted Core or a reconnect rebinds
// the same member.
var (
	sessionMetaKeys = []string{"session_id", "grok_session_id", "conversation_id", "mcp_session_id"}
	sessionEnvKeys  = []string{"GROK_SESSION_ID", "XAI_SESSION_ID"}
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

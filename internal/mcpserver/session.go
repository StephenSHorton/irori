package mcpserver

import (
	"crypto/sha256"
	"encoding/hex"
	"strings"
)

// Session id precedence for irori_join (suzuri.md §3): the call's _meta, then
// the environment, then one derived from the MCP client's name and the first
// join name (namedSession), fixed for the process. The Core gives the same
// session id the same memberKey, so a restarted Core, a reconnect or a new
// `irori mcp` process joining under the same name rebinds the same member,
// and a script that posts a card and later updates it keeps the right to.
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

// namedSession is the session id of a member without one of its own: a
// hash of the MCP client's name and the join name, so every process that
// joins under that name from that client is the same member. Two live
// processes doing so share it: the later join takes over (SupersededText).
func namedSession(client, name string) string {
	sum := sha256.Sum256([]byte(strings.TrimSpace(client) + "\x00" + strings.TrimSpace(name)))
	return "named_" + hex.EncodeToString(sum[:12])
}

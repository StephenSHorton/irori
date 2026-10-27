// Package attach is the wire between the Core and the processes that wake
// agents: `irori attach` (spawned by the Irori Claude Code mod) and a bound
// `irori mcp` (Grok). See docs/design/v0.2-talk.md §7 and §9.
//
// The Core serves GET /v1/attach?session=<id> (mod) or ?member=<secret>
// (Grok) as NDJSON: one Event per line, plus a bare "\n" keepalive every 15 s
// that readers skip. `irori attach` copies events to its stdout unchanged.
//
// CONTRACT (types are shared by the api, mcpserver and the mod; keep them):
package attach

// Protocol version carried in Hello.V.
const Version = 1

// Event types.
const (
	TypeHello      = "hello"
	TypeDeliver    = "deliver"
	TypeHeld       = "held"
	TypeMoved      = "moved"
	TypeInbox      = "inbox"
	TypeSuperseded = "superseded"
	TypeError      = "error"
)

// Event is one NDJSON line. Only the fields for its Type are set.
type Event struct {
	Type string `json:"type"`

	// hello
	V            int    `json:"v,omitempty"`
	Session      string `json:"session,omitempty"`
	AgentKey     string `json:"agentKey,omitempty"`
	Name         string `json:"name,omitempty"`
	API          *API   `json:"api,omitempty"`
	AttachSecret string `json:"attachSecret,omitempty"`

	// deliver: the Core has applied policy and built the fenced prompt.
	WakeID      string   `json:"wakeId,omitempty"`
	Token       string   `json:"token,omitempty"`
	Reminder    bool     `json:"reminder,omitempty"`
	DeliveryIDs []string `json:"deliveryIds,omitempty"`
	MessageIDs  []string `json:"messageIds,omitempty"`
	Prompt      string   `json:"prompt,omitempty"`
	From        string   `json:"from,omitempty"` // first sender's name, for toasts / channel meta

	// held / moved
	DeliveryID string `json:"deliveryId,omitempty"`
	Why        string `json:"why,omitempty"`
	Preview    string `json:"preview,omitempty"` // first 120 runes, single line
	To         string `json:"to,omitempty"`

	// inbox
	Unread int `json:"unread,omitempty"`
	HeldN  int `json:"held,omitempty"`

	// error
	Message string `json:"message,omitempty"`
}

// API tells the mod where to send control calls.
type API struct {
	URL   string `json:"url"`   // http://127.0.0.1:<port>
	Token string `json:"token"` // bearer token
}

// Headers on control and tool calls.
const (
	HeaderAttach = "X-Irori-Attach" // AttachSecret from hello (mod)
	HeaderMember = "X-Irori-Member" // member secret from irori_join (Grok)
	HeaderOrigin = "X-Irori-Origin" // "composer" when a person typed the /irori command
)

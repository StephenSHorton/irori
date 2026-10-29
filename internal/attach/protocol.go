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

// Protocol version carried in Hello.V. 2 adds the append event (next-turn
// delivery, docs/design/board.md "Contracts"; spike 04).
const Version = 2

// AppendSince is the first mod version (the mod= query, MOD_VERSION in
// mod/hooks/lib.ts) that handles append events. The Core sends append only
// to an attach whose mod version is at least this (a hand-rolled compare of
// three dot-separated ints, any suffix ignored; no new dependency); an
// older or unnamed mod gets its next_turn items riding its next deliver, so
// nothing sits unacked. The Grok path never gets append.
const AppendSince = "0.4.0"

// Event types.
const (
	TypeHello   = "hello"
	TypeDeliver = "deliver"
	// TypeAppend is a next-turn delivery for the mod (inbox.NextAppend). It
	// carries WakeID, Token ("[irori-append:<id>]"), DeliveryIDs,
	// MessageIDs, Prompt (the fenced text, built by the Core; each item has
	// mode="next_turn", except a wake item the mod was handed
	// inbox.MaxWakeTries times without confirming one, card #23), From, Mode
	// "next_turn" and Overflow.
	//
	// Mod contract (mod/hooks/register.tsx):
	//  1. Dedupe on Token against the tracked appends (a reattach re-sends an
	//     append not yet confirmed, with the same token). A known token that
	//     isn't confirmed only re-POSTs delivered.
	//  2. Track it ({token, deliveryIds, delivered:false, engaged:false})
	//     before calling the host, like a wake.
	//  3. $.session.append({ message: { type: 'user', content: [{ type:
	//     'text', text: prompt }] } }). It starts no turn and the row is
	//     stored as meta (isMeta), so it is never shown as typed by the
	//     person. No toast: next-turn delivery is meant to be quiet.
	//  4. When it resolves: POST /v1/deliveries/delivered {token}. If it
	//     throws: log to debug and forget the token, so the Core's re-send
	//     (reattach, or the reclaim after ReattachGrace) appends it again.
	//  5. On the next main-loop turn.start (e.agentId undefined) after step 4,
	//     whatever started it: POST /v1/deliveries/engaged {token, turnId}
	//     for every delivered, not yet engaged append. The append's token is
	//     not in e.text, so this is not a token match. Never POST turn-ended
	//     for an append.
	TypeAppend = "append"
	// TypeName says the session's name changed (card #21): Name,
	// NameSource and AITitle, as in hello. Sent by the keepalive tick when
	// any of them differs from what the stream last sent. The mod sets
	// Claude's session title from it; an older mod ignores it.
	TypeName       = "name"
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
	V        int    `json:"v,omitempty"`
	Session  string `json:"session,omitempty"`
	AgentKey string `json:"agentKey,omitempty"`
	Name     string `json:"name,omitempty"`
	// NameSource and AITitle (hello and name events, Claude sessions): where
	// Name came from ("user", "irori" or "", presence.AgentCard.NameSource)
	// and the transcript's newest ai-title.
	NameSource   string `json:"nameSource,omitempty"`
	AITitle      string `json:"aiTitle,omitempty"`
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
	// Mode is "next_turn" on an append, and on a Grok deliver that holds
	// only next_turn items (irori mcp puts it in the channel meta as
	// mode=next_turn); "" on every other deliver.
	Mode string `json:"mode,omitempty"`
	// Overflow lists the DeliveryIDs that didn't fit in Prompt (16 KB): the
	// prompt names them in one line for the inbox tool. They are part of
	// DeliveryIDs and are confirmed with the rest.
	Overflow []string `json:"overflow,omitempty"`

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

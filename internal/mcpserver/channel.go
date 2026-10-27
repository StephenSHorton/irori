package mcpserver

import (
	"context"
	"encoding/json"
	"errors"
	"regexp"
	"strings"
	"sync"

	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/StephenSHorton/irori/internal/attach"
)

// ChannelMethod is the notification Grok (and Claude Code with channels on)
// turns into a new turn. grok-fork also accepts notifications/x.ai/channel.
const ChannelMethod = "notifications/claude/channel"

// ChannelCapability is the experimental capability key Claude Code checks.
const ChannelCapability = "claude/channel"

// ChannelParams is the notification's params.
type ChannelParams struct {
	Content string            `json:"content"`
	Meta    map[string]string `json:"meta"`
}

var metaKeyRE = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]*$`)

// ValidMetaKey reports whether key survives Grok's and Claude Code's channel
// meta filter (anything else is silently dropped by the client).
func ValidMetaKey(key string) bool { return metaKeyRE.MatchString(key) }

// maxMetaValueRunes bounds one meta value; they become tag attributes.
const maxMetaValueRunes = 400

// cleanMeta drops keys clients would drop, folds values to one line and never
// returns nil (clients expect an object).
func cleanMeta(in map[string]string) map[string]string {
	out := make(map[string]string, len(in))
	for k, v := range in {
		if !ValidMetaKey(k) {
			continue
		}
		out[k] = oneLine(v, maxMetaValueRunes)
	}
	return out
}

// channelParams shapes a deliver event as a channel push (design §9). The
// prompt is the Core's fenced text, passed through unchanged.
func channelParams(ev attach.Event) ChannelParams {
	meta := map[string]string{"trust": "untrusted"}
	if len(ev.DeliveryIDs) > 0 {
		meta["delivery_id"] = ev.DeliveryIDs[0]
		meta["delivery_ids"] = strings.Join(ev.DeliveryIDs, ",")
	}
	if len(ev.MessageIDs) > 0 {
		meta["message_id"] = ev.MessageIDs[0]
	}
	if ev.From != "" {
		meta["from"] = ev.From
	}
	if ev.Reminder {
		meta["reminder"] = "true"
	}
	if ev.Mode != "" {
		meta["mode"] = ev.Mode // "next_turn": no need to drop what it is doing
	}
	return ChannelParams{Content: ev.Prompt, Meta: cleanMeta(meta)}
}

// notifyTransport wraps the real transport and keeps the connection so the
// server can write a notification the go-sdk has no API for. ioConn.Write
// holds the connection's write mutex, so a raw write never interleaves with
// an SDK response on stdout.
type notifyTransport struct {
	inner mcp.Transport

	mu   sync.Mutex
	conn mcp.Connection
}

func (t *notifyTransport) Connect(ctx context.Context) (mcp.Connection, error) {
	c, err := t.inner.Connect(ctx)
	if err != nil {
		return nil, err
	}
	t.mu.Lock()
	t.conn = c
	t.mu.Unlock()
	return c, nil
}

var errNoConnection = errors.New("mcp connection not open")

// notify writes one JSON-RPC notification (no id).
func (t *notifyTransport) notify(ctx context.Context, method string, params any) error {
	t.mu.Lock()
	c := t.conn
	t.mu.Unlock()
	if c == nil {
		return errNoConnection
	}
	raw, err := json.Marshal(params)
	if err != nil {
		return err
	}
	return c.Write(ctx, &jsonrpc.Request{Method: method, Params: raw})
}

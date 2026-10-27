package core

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"sort"
	"sync"
	"time"

	"github.com/StephenSHorton/irori/internal/api"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/presence"
)

// KindMemberJoined is logged when a Grok member binds: {key, name}. The
// member secret is never logged.
const KindMemberJoined = "member.joined"

// member is one bound Grok member. Secrets live only in memory: an Irori
// restart makes every `irori mcp` join again.
type member struct {
	key, name, secret string
	joinedAt          time.Time
	open              int       // open member streams, counted per stream (2 while a supersede settles)
	lastSeen          time.Time // join, or the last stream open or close
	streamed          bool      // a member stream has opened at least once
}

type members struct {
	mu       sync.Mutex
	deviceID string
	grace    time.Duration
	now      func() time.Time
	byKey    map[string]*member
	bySecret map[string]*member
}

func newMembers(deviceID string) *members {
	return &members{
		deviceID: deviceID, grace: inbox.ReattachGrace, now: time.Now,
		byKey: map[string]*member{}, bySecret: map[string]*member{},
	}
}

func randHex(n int) (string, error) {
	b := make([]byte, n)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return hex.EncodeToString(b), nil
}

// memberID is stable for a client session id, so a rejoin (an `irori mcp`
// restart for the same Grok session) keeps its key, across Irori restarts too.
func (m *members) memberID(session string) (string, error) {
	if session == "" {
		return randHex(6)
	}
	sum := sha256.Sum256([]byte(m.deviceID + "\x00" + session))
	return hex.EncodeToString(sum[:6]), nil
}

func (m *members) join(name, session string) (api.Member, error) {
	id, err := m.memberID(session)
	if err != nil {
		return api.Member{}, err
	}
	secret, err := randHex(24)
	if err != nil {
		return api.Member{}, err
	}
	key := m.deviceID + "/grok-" + id
	m.mu.Lock()
	defer m.mu.Unlock()
	now := m.now()
	rec := m.byKey[key]
	if rec == nil {
		rec = &member{key: key, joinedAt: now, lastSeen: now}
		m.byKey[key] = rec
	} else {
		delete(m.bySecret, rec.secret) // a rejoin retires the old secret
	}
	// A join alone lists no card (and a rejoin doesn't extend the grace):
	// the card shows while a stream is open and for the grace after it closes.
	rec.name, rec.secret = name, secret
	m.bySecret[secret] = rec
	return api.Member{Key: key, Secret: secret, Name: name}, nil
}

func (m *members) bySecretLookup(secret string) (api.Member, bool) {
	if secret == "" {
		return api.Member{}, false
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	rec := m.bySecret[secret]
	if rec == nil {
		return api.Member{}, false
	}
	return api.Member{Key: rec.key, Secret: rec.secret, Name: rec.name}, true
}

func (m *members) stream(key string, open bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	rec := m.byKey[key]
	if rec == nil {
		return
	}
	if open {
		rec.open++
		rec.streamed = true
	} else if rec.open > 0 {
		rec.open--
	}
	rec.lastSeen = m.now()
}

// cards are the synthetic presence cards for members with an open stream,
// or whose last stream closed less than the grace ago. A member that joined
// but never streamed has no card.
func (m *members) cards() []presence.AgentCard {
	m.mu.Lock()
	defer m.mu.Unlock()
	now := m.now()
	var out []presence.AgentCard
	for _, rec := range m.byKey {
		if rec.open == 0 && (!rec.streamed || now.Sub(rec.lastSeen) >= m.grace) {
			continue
		}
		sid := rec.key[len(m.deviceID)+1:]
		out = append(out, presence.AgentCard{
			Key: rec.key, DeviceID: m.deviceID, Agent: "grok", SessionID: sid,
			Name: rec.name, Status: "idle", SessionKind: "member", PeerFeatures: []string{},
			StartedAt: rec.joinedAt, UpdatedAt: rec.lastSeen, LastActivity: rec.lastSeen,
		})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Key < out[j].Key })
	return out
}

// JoinMember implements api.Backend.
func (c *Core) JoinMember(ctx context.Context, name, session string) (api.Member, error) {
	mb, err := c.members.join(name, session)
	if err != nil {
		return api.Member{}, err
	}
	c.addSecret(mb.Secret)
	if _, err := c.store.Append(ctx, "irori", KindMemberJoined, map[string]any{
		"key": mb.Key, "name": mb.Name,
	}); err != nil {
		return api.Member{}, err
	}
	return mb, nil
}

// MemberBySecret implements api.Backend.
func (c *Core) MemberBySecret(secret string) (api.Member, bool) {
	return c.members.bySecretLookup(secret)
}

// MemberStream implements api.Backend.
func (c *Core) MemberStream(key string, open bool) { c.members.stream(key, open) }

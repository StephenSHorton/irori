package core

import (
	"context"
	"crypto/ed25519"
	"database/sql"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/store"
)

// writeV03Log builds what v0.3.2 leaves in a data dir: device.json without
// keys, and irori.db at schema v1 holding a realistic history (starts,
// sessions, a message and its delivery, a task, policy changes and a Grok
// member) with no signatures.
func writeV03Log(t *testing.T, data string) device.Identity {
	t.Helper()
	dev, err := device.LoadOrCreate(data)
	if err != nil {
		t.Fatal(err)
	}
	sdb, err := sql.Open("sqlite", filepath.Join(data, "irori.db")+"?_pragma=journal_mode(WAL)")
	if err != nil {
		t.Fatal(err)
	}
	defer sdb.Close()
	for _, q := range []string{
		`CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL DEFAULT (unixepoch()))`,
		`CREATE TABLE events (
			device_id TEXT NOT NULL, seq INTEGER NOT NULL, hlc INTEGER NOT NULL,
			actor TEXT NOT NULL, kind TEXT NOT NULL, payload BLOB,
			PRIMARY KEY (device_id, seq)
		) WITHOUT ROWID`,
		`CREATE INDEX events_hlc ON events (hlc)`,
		`CREATE INDEX events_kind ON events (kind, hlc)`,
		`INSERT INTO schema_migrations (version) VALUES (1)`,
	} {
		if _, err := sdb.Exec(q); err != nil {
			t.Fatal(err)
		}
	}
	d := dev.ID
	alpha := d + "/aaaaaaaa-1111-4111-8111-111111111111"
	beta := d + "/bbbbbbbb-2222-4222-8222-222222222222"
	at := "2026-10-20T10:00:00Z"
	rows := []struct{ actor, kind, payload string }{
		{"irori", "device.started", `{"name":"pc","os":"windows","version":"0.3.2","headless":false}`},
		{"irori", "agent.seen", fmt.Sprintf(`{"key":%q,"deviceId":%q,"agent":"claude","sessionId":"aaaaaaaa-1111-4111-8111-111111111111","name":"alpha","status":"idle"}`, alpha, d)},
		{"irori", "agent.seen", fmt.Sprintf(`{"key":%q,"deviceId":%q,"agent":"claude","sessionId":"bbbbbbbb-2222-4222-8222-222222222222","name":"beta","status":"idle"}`, beta, d)},
		{"user", "policy.set", `{"mode":"everyone","block":[],"allow":[],"paused":false,"by":"user:` + d + `","at":"` + at + `"}`},
		{"agent:" + alpha, "message.sent", fmt.Sprintf(`{"id":"msg_1","from":{"key":%q,"name":"alpha","kind":"claude"},"to":{"kind":"agent","value":%q},"text":"hello from v0.3","trust":"untrusted","sentAt":%q}`, alpha, beta, at)},
		{"agent:" + alpha, "delivery.state", fmt.Sprintf(`{"deliveryId":"del_1","messageId":"msg_1","to":%q,"state":"queued","by":%q,"at":%q}`, beta, alpha, at)},
		{"irori", "member.joined", fmt.Sprintf(`{"key":"%s/grok-1","name":"grokky"}`, d)},
		{"agent:" + alpha, "task.created", fmt.Sprintf(`{"taskId":"t_v03","title":"carried over","by":%q,"at":%q}`, alpha, at)},
		{"user", "policy.set", `{"mode":"allowlist","block":["gamma"],"allow":["alpha"],"paused":true,"by":"user:` + d + `","at":"` + at + `"}`},
	}
	base := time.Date(2026, 10, 20, 10, 0, 0, 0, time.UTC).UnixMilli()
	for i, r := range rows {
		if _, err := sdb.Exec(`INSERT INTO events (device_id, seq, hlc, actor, kind, payload) VALUES (?, ?, ?, ?, ?, ?)`,
			d, i+1, (base+int64(i))<<16, r.actor, r.kind, []byte(r.payload)); err != nil {
			t.Fatal(err)
		}
	}
	return dev
}

func TestStartMigratesAV03DataDir(t *testing.T) {
	data, claude := t.TempDir(), t.TempDir()
	dev := writeV03Log(t, data)
	start := func() *Core {
		t.Helper()
		c, err := Start(context.Background(), Options{DataDir: data, ClaudeDir: claude, Version: "0.4.0-test", Headless: true, Keystore: identity.FileKeystore{}})
		if err != nil {
			t.Fatal(err)
		}
		return c
	}
	c := start()
	ctx := context.Background()

	if c.Device().ID != dev.ID || c.Device().DevicePub == "" || c.Device().PersonID != c.Person().ID {
		t.Fatalf("device after migration %+v", c.Device())
	}
	if _, err := os.Stat(filepath.Join(data, "irori.db"+store.BackupSuffix)); err != nil {
		t.Fatalf("no v0.3 backup: %v", err)
	}
	// The last v0.3 policy survives the move to local_events.
	pol := c.Inbox().Policy()
	if pol.Mode != inbox.Allowlist || !pol.Paused || len(pol.Block) != 1 || pol.Block[0] != "gamma" {
		t.Fatalf("policy after migration %+v", pol)
	}
	// Folds see the old history.
	if _, ok := c.Coord().GetTask("t_v03"); !ok {
		t.Fatal("v0.3 task missing")
	}
	if d, ok := c.Inbox().Get("del_1"); !ok || d.MessageID != "msg_1" {
		t.Fatalf("v0.3 delivery %+v %v", d, ok)
	}
	// Every own row is signed by the device key, and no local kind is left
	// in the log.
	pub, _ := hex.DecodeString(c.Device().DevicePub)
	evs, err := c.store.Missing(ctx, store.Vector{}, store.MaxMissingLimit)
	if err != nil {
		t.Fatal(err)
	}
	moved := 0
	for _, ev := range evs {
		if !ed25519.Verify(pub, store.SigningBytes(ev), ev.Sig) {
			t.Fatalf("row %d (%s) not signed by the device key", ev.Seq, ev.Kind)
		}
		if store.IsLocal(ev.Kind) {
			t.Fatalf("local kind %s left in events", ev.Kind)
		}
		if ev.Kind == store.KindLocalMoved {
			moved++
		}
	}
	if moved != 3 {
		t.Fatalf("%d local.moved placeholders, want 3", moved)
	}
	// The root certified itself; its owner key is the person, a made-up one isn't.
	if !c.IsOwner("user:"+dev.ID) || c.IsOwner("user:someone-else") {
		t.Fatal("IsOwner wrong after migration")
	}
	if ds := c.CertifiedDevices(); len(ds) != 1 || ds[0].Cert.DeviceID != dev.ID || ds[0].Cert.Role != identity.RoleNode {
		t.Fatalf("certified devices %+v", ds)
	}
	n := len(evs)
	c.Close()

	// A second start migrates nothing again and writes no second self-cert.
	c2 := start()
	defer c2.Close()
	evs2, _ := c2.store.Missing(ctx, store.Vector{}, store.MaxMissingLimit)
	certs := 0
	for _, ev := range evs2 {
		if ev.Kind == store.KindPersonDeviceAdded {
			certs++
		}
	}
	// Only the new device.started row is added.
	if len(evs2) != n+1 || certs != 1 {
		t.Fatalf("second start: %d rows (want %d), %d certs", len(evs2), n+1, certs)
	}
	if pol := c2.Inbox().Policy(); pol.Mode != inbox.Allowlist || !pol.Paused {
		t.Fatalf("policy after restart %+v", pol)
	}
	if _, ok := c2.Coord().GetTask("t_v03"); !ok {
		t.Fatal("task missing after restart")
	}
}

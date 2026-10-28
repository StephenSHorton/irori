package roomnet

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	room "github.com/StephenSHorton/go-room"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/store"
)

// attach stores size random bytes in d's attachment store and offers them.
func (d *dev) attach(size int) (blob.FileRef, []byte) {
	d.t.Helper()
	data := make([]byte, size)
	rand.Read(data)
	p := filepath.Join(d.t.TempDir(), "file.bin")
	if err := os.WriteFile(p, data, 0o600); err != nil {
		d.t.Fatal(err)
	}
	ref, err := d.files.PutFile(p)
	if err != nil {
		d.t.Fatal(err)
	}
	ref.Device = d.ident.Device.ID
	if err := d.node.Offer(context.Background(), ref); err != nil {
		d.t.Fatal(err)
	}
	return ref, data
}

func (d *dev) offered(sha string) bool {
	evs, _ := d.st.List(context.Background(), store.Filter{Kinds: []string{store.KindAttachOffered}, Limit: store.MaxListLimit})
	for _, ev := range evs {
		if strings.Contains(string(ev.Payload), sha) {
			return true
		}
	}
	return false
}

// Attachments cross a real go-room connection both ways on blob streams
// while a backfill fills the sync channel (§10, "go-room under Irori").
func TestAttachmentsCrossGoRoomBothWaysDuringABackfill(t *testing.T) {
	ctx := context.Background()
	hub := room.NewMemHub()
	a := newDev(t, "pc", "")
	b := newDev(t, "mac", a.dir)
	add, err := a.ident.CertFor(b.ident.Device.ID, hex.EncodeToString(b.ident.DevicePub()), "mac", identity.RoleDesktop, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := a.st.Append(ctx, "irori", store.KindPersonDeviceAdded, add); err != nil {
		t.Fatal(err)
	}
	person, _ := a.st.List(ctx, store.Filter{Kinds: []string{"person."}})
	for range 2 {
		for _, ev := range person {
			_ = b.chain.Learn(ev)
		}
	}
	a.start(hub)
	b.start(hub)
	fa, da := a.attach(4<<20 + 7)
	fb, db := b.attach(2<<20 + 3)
	eventually(t, "the offers cross", 60*time.Second, func() bool { return a.offered(fb.SHA256) && b.offered(fa.SHA256) })

	// A backfill a's way and b's way while both fetch.
	big := strings.Repeat("z", 8000)
	var lastA, lastB store.Event
	for i := range 400 {
		lastA, _ = a.st.Append(ctx, "irori", "capture.turn", map[string]any{"i": i, "text": big})
		lastB, _ = b.st.Append(ctx, "irori", "capture.turn", map[string]any{"i": i, "text": big})
	}
	errs := make(chan error, 2)
	go func() { _, err := b.node.FetchFile(ctx, fa); errs <- err }()
	go func() { _, err := a.node.FetchFile(ctx, fb); errs <- err }()
	for range 2 {
		select {
		case err := <-errs:
			if err != nil {
				t.Fatalf("fetch: %v", err)
			}
		case <-time.After(60 * time.Second):
			t.Fatal("a fetch hung")
		}
	}
	eventually(t, "both backfills land", 60*time.Second, func() bool { return a.has(lastB) && b.has(lastA) })
	read := func(d *dev, ref blob.FileRef) []byte {
		f, err := d.files.Open(ref.SHA256)
		if err != nil {
			t.Fatalf("%s doesn't hold it: %v", d.name, err)
		}
		defer f.Close()
		var buf bytes.Buffer
		buf.ReadFrom(f)
		return buf.Bytes()
	}
	if !bytes.Equal(read(b, fa), da) || !bytes.Equal(read(a, fb), db) {
		t.Fatal("a fetched copy differs")
	}
}

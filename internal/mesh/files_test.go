package mesh

import (
	"bytes"
	"context"
	"crypto/rand"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/store"
)

// attachFile writes size random bytes to a file, stores it in d's
// attachment store and offers it, as attaching does.
func (d *dev) attachFile(name string, size int) (blob.FileRef, []byte) {
	d.t.Helper()
	data := make([]byte, size)
	rand.Read(data)
	p := filepath.Join(d.t.TempDir(), name)
	if err := os.WriteFile(p, data, 0o600); err != nil {
		d.t.Fatal(err)
	}
	ref, err := d.files.PutFile(p)
	if err != nil {
		d.t.Fatal(err)
	}
	ref.Device = d.id()
	if err := d.node.Offer(context.Background(), ref); err != nil {
		d.t.Fatal(err)
	}
	return ref, data
}

func (d *dev) stored(ref blob.FileRef) []byte {
	d.t.Helper()
	f, err := d.files.Open(ref.SHA256)
	if err != nil {
		d.t.Fatalf("%s doesn't hold %s: %v", d.name, ref.Name, err)
	}
	defer f.Close()
	var b bytes.Buffer
	b.ReadFrom(f)
	return b.Bytes()
}

// TestAnAttachmentIsFetchedFromTheOriginThenFromASecondHolder is §10's
// "an attachment fetched from the origin, then from a second holder with
// the origin gone, while live rows keep flowing on the sync stream".
func TestAnAttachmentIsFetchedFromTheOriginThenFromASecondHolder(t *testing.T) {
	net := NewFakeNet()
	pc, mac, nd := trio(t, net)
	ctx := context.Background()
	ref, data := pc.attachFile("design.pdf", 3<<20+123)
	eventually(t, "the offer replicates", converged(pc, mac, nd))

	from, err := mac.node.FetchFile(ctx, ref)
	if err != nil || from != pc.id() {
		t.Fatalf("mac fetch = %q, %v", from, err)
	}
	if !bytes.Equal(mac.stored(ref), data) {
		t.Fatal("mac's copy differs")
	}
	// A second fetch is served from the store.
	if from, err := mac.node.FetchFile(ctx, ref); err != nil || from != "" {
		t.Fatalf("second fetch = %q, %v", from, err)
	}
	// The mac now offers it too.
	eventually(t, "the mac's offer reaches the node", func() bool {
		hs, _ := nd.node.holders(ctx, ref.SHA256)
		return len(hs) == 2
	})

	// The origin goes away; the node fetches from the mac while the mac
	// keeps writing rows.
	pc.stop()
	net.Isolate(pc.id())
	done := make(chan error, 1)
	go func() {
		f, err := nd.node.FetchFile(ctx, ref)
		if err == nil && f != mac.id() {
			err = errors.New("fetched from " + f)
		}
		done <- err
	}()
	var live []store.Event
	for i := range 40 {
		live = append(live, mac.append("capture.turn", map[string]any{"i": i}))
	}
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("node fetch: %v", err)
		}
	case <-time.After(20 * time.Second):
		t.Fatal("node fetch hung")
	}
	eventually(t, "the mac's live rows reach the node", func() bool { return nd.has(live[len(live)-1]) })
	if !bytes.Equal(nd.stored(ref), data) {
		t.Fatal("node's copy differs")
	}

	// With every holder away the fetch says where the file is.
	other, _ := mac.attachFile("notes.txt", 1000)
	eventually(t, "the second offer reaches the node", func() bool {
		hs, _ := nd.node.holders(ctx, other.SHA256)
		return len(hs) == 1
	})
	mac.stop()
	var off *OfflineError
	if _, err := nd.node.FetchFile(ctx, other); !errors.As(err, &off) || len(off.Holders) != 1 || off.Holders[0] != mac.id() {
		t.Fatalf("all holders away = %v", err)
	}
}

func TestAFetchResumesAndOnlyOfferedFilesAreServed(t *testing.T) {
	net := NewFakeNet()
	pc, mac, _ := trio(t, net)
	ctx := context.Background()
	ref, data := pc.attachFile("log.txt", 200<<10)
	eventually(t, "the offer replicates", func() bool {
		hs, _ := mac.node.holders(ctx, ref.SHA256)
		return len(hs) == 1
	})

	// Half of it is already here from a cut stream: the fetch asks for the
	// rest only.
	part, _, err := mac.files.Partial(ref.SHA256)
	if err != nil {
		t.Fatal(err)
	}
	part.Write(data[:100<<10])
	part.Close()
	if _, err := mac.node.FetchFile(ctx, ref); err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(mac.stored(ref), data) {
		t.Fatal("resumed copy differs")
	}

	// A file the pc holds but no row references is never served.
	p := filepath.Join(t.TempDir(), "private.txt")
	os.WriteFile(p, []byte("not offered"), 0o600)
	hidden, err := pc.files.PutFile(p)
	if err != nil {
		t.Fatal(err)
	}
	hidden.Device = pc.id()
	if _, err := mac.node.FetchFile(ctx, hidden); err == nil || mac.files.Has(hidden.SHA256) {
		t.Fatalf("an unreferenced file was fetched: %v", err)
	}

	// A ref that claims more than the cap is refused before anything is
	// asked for; one whose length is wrong is never kept.
	big := ref
	big.Size = blob.MaxFileSize + 1
	if _, err := mac.node.FetchFile(ctx, big); !errors.Is(err, blob.ErrTooLarge) {
		t.Fatalf("over the cap = %v", err)
	}
	other, _ := pc.attachFile("other.bin", 5000)
	eventually(t, "the offer replicates", func() bool {
		hs, _ := mac.node.holders(ctx, other.SHA256)
		return len(hs) == 1
	})
	short := other
	short.Size = 4000
	if _, err := mac.node.FetchFile(ctx, short); err == nil || mac.files.Has(other.SHA256) {
		t.Fatalf("a wrong length was kept: %v", err)
	}
}

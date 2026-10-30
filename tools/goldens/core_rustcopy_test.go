package core

// Injected into I:internal/core with `go test -overlay` by
// tools/goldens/go-opens-rust.sh (architecture.md §8 M2: "after a Rust
// append on a copy, the Go build still opens that copy"). It is not
// `irori serve --headless` on purpose: the copy holds this PC's real device
// identity, and a real transport would sync it with the person's other
// devices as a second copy of this device, forking its history. Here the
// whole Go Core starts on the copy with a FakeNet (no hub, no home server,
// no LAN socket) and an empty Claude dir.

import (
	"bufio"
	"context"
	"os"
	"strconv"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/store"
)

func TestGoOpensRustCopy(t *testing.T) {
	home, manifest := os.Getenv("IRORI_RUST_HOME"), os.Getenv("IRORI_RUST_MANIFEST")
	if home == "" || manifest == "" {
		t.Skip("IRORI_RUST_HOME / IRORI_RUST_MANIFEST not set")
	}
	f, err := os.Open(manifest)
	if err != nil {
		t.Fatal(err)
	}
	sc := bufio.NewScanner(f)
	if !sc.Scan() {
		t.Fatal("empty manifest")
	}
	fields := strings.Split(sc.Text(), "\t")
	f.Close()
	if len(fields) != 3 {
		t.Fatalf("manifest line has %d fields", len(fields))
	}
	dev, hash := fields[0], fields[2]
	seq, err := strconv.ParseInt(fields[1], 10, 64)
	if err != nil {
		t.Fatal(err)
	}

	net := mesh.NewFakeNet()
	c, err := Start(context.Background(), Options{DataDir: home, ClaudeDir: t.TempDir(), Version: "0.5.0-rust-check",
		Headless: true, Net: net})
	if err != nil {
		t.Fatalf("Go Core did not start on the Rust copy: %v", err)
	}
	defer c.Close()
	ctx := context.Background()
	if c.device.ID != dev {
		t.Fatalf("Go opened device %s…, Rust appended as %s…", c.device.ID[:8], dev[:8])
	}
	evs, err := c.store.List(ctx, store.Filter{DeviceID: dev, AfterSeq: seq - 1, Limit: 1})
	if err != nil {
		t.Fatal(err)
	}
	if len(evs) != 1 || evs[0].Seq != seq {
		t.Fatalf("Go does not hold Rust's row %d", seq)
	}
	ev := evs[0]
	if got := store.RowHash(ev); got != hash {
		t.Fatalf("Rust's row %d: Go RowHash differs", seq)
	}
	if err := c.chain.VerifyRow(ev); err != nil {
		t.Fatalf("Rust's row %d does not verify in Go: %v", seq, err)
	}
	vec, err := c.store.Vector(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if vec[dev] <= seq {
		t.Fatalf("Go's own start row did not follow Rust's: vector %d, Rust row %d", vec[dev], seq)
	}
	after, err := c.store.List(ctx, store.Filter{DeviceID: dev, AfterSeq: seq, Limit: 10})
	if err != nil {
		t.Fatal(err)
	}
	for _, e := range after {
		if err := c.chain.VerifyRow(e); err != nil {
			t.Fatalf("Go's own row %d after Rust's does not verify: %v", e.Seq, err)
		}
	}
	total := 0
	for _, n := range vec {
		total += int(n)
	}
	t.Logf("Go Core started on the Rust copy: Rust row %d verifies (RowHash equal), Go appended %d rows after it (own seq now %d), %d devices, %d rows by vector, net joins %d",
		seq, len(after), vec[dev], len(c.Mesh().Devices(false)), total, net.Joins())
}

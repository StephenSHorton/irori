// Golden exporter for crates/hlc (store.md §2.6, §6 "hlc").
//
// Injected into I:internal/hlc by tools/goldens/export.sh with
// `go test -overlay`; runs only when IRORI_GOLDEN_OUT is set. Each script is a
// list of steps against one clock with an injected wall time; every step
// records the clock's answer, so the Rust replay must match step for step.
package hlc_test

import (
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
)

type hlcStep struct {
	Op     string `json:"op"`            // "wall", "now", "observe", "observeBounded"
	WallMs int64  `json:"wallMs"`        // op "wall": the injected clock's UnixMilli
	A      uint64 `json:"a,omitempty"`   // observe arg / observeBounded remote
	B      uint64 `json:"b,omitempty"`   // observeBounded max
	Out    uint64 `json:"out,omitempty"` // op "now": the returned timestamp
}

type hlcDecode struct {
	T       uint64 `json:"t"`
	WallMs  int64  `json:"wallMs"`
	Logical uint16 `json:"logical"`
}

func TestGoldenHLC(t *testing.T) {
	goldenDir(t)
	scripts := map[string][]hlcStep{
		"stall": {
			{Op: "wall", WallMs: 1_700_000_000_000}, {Op: "now"}, {Op: "now"}, {Op: "now"},
			{Op: "wall", WallMs: 1_700_000_000_001}, {Op: "now"}, {Op: "wall", WallMs: 1_699_999_999_000}, {Op: "now"}, {Op: "now"},
		},
		"bounded": {
			{Op: "wall", WallMs: 1_700_000_000_000},
			{Op: "observeBounded", A: uint64(1_700_086_400_000) << 16, B: uint64(1_700_000_120_000) << 16},
			{Op: "now"}, {Op: "observeBounded", A: 5, B: uint64(1_700_000_120_000) << 16}, {Op: "now"},
			{Op: "observeBounded", A: uint64(1_700_000_120_000)<<16 + 9, B: uint64(1_700_000_120_000)<<16 + 3}, {Op: "now"},
		},
		"observe": {
			{Op: "wall", WallMs: 1000}, {Op: "observe", A: 5000 << 16}, {Op: "now"}, {Op: "observe", A: 7}, {Op: "now"},
			{Op: "observe", A: 1 << 62}, {Op: "now"}, {Op: "observe", A: 1<<64 - 1}, {Op: "now"}, {Op: "now"},
		},
		"logicalCarry": {
			{Op: "wall", WallMs: 1}, {Op: "observe", A: 1<<16 + 0xfffe}, {Op: "now"}, {Op: "now"}, {Op: "now"},
		},
		"preEpoch": {
			{Op: "wall", WallMs: -1}, {Op: "now"}, {Op: "now"}, {Op: "wall", WallMs: -1_000_000}, {Op: "now"},
			{Op: "wall", WallMs: 0}, {Op: "now"},
		},
		"wallJumpsAhead": {
			{Op: "wall", WallMs: 1_700_000_000_000}, {Op: "now"}, {Op: "wall", WallMs: 1_800_000_000_000}, {Op: "now"}, {Op: "now"},
		},
	}
	for _, steps := range scripts {
		var wall int64
		c := hlc.New(func() time.Time { return time.UnixMilli(wall) })
		for i := range steps {
			s := &steps[i]
			switch s.Op {
			case "wall":
				wall = s.WallMs
			case "now":
				s.Out = uint64(c.Now())
			case "observe":
				c.Observe(hlc.Timestamp(s.A))
			case "observeBounded":
				c.ObserveBounded(hlc.Timestamp(s.A), hlc.Timestamp(s.B))
			}
		}
	}
	var decodes []hlcDecode
	for _, v := range []uint64{0, 1, 0xffff, 0x10000, uint64(1_700_000_000_000)<<16 | 7, 1 << 62, 1<<63 + 5, 1<<64 - 1} {
		ts := hlc.Timestamp(v)
		decodes = append(decodes, hlcDecode{v, ts.Wall().UnixMilli(), ts.Logical()})
	}
	writeGolden(t, "hlc/hlc.json", map[string]any{"scripts": scripts, "decode": decodes})
}

package decision

import (
	"fmt"
	"math"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

// checkWithin runs Check and fails if it panics or takes longer than d.
func checkWithin(t *testing.T, a Anchor, root string, d time.Duration) Status {
	t.Helper()
	type res struct {
		s   Status
		err any
	}
	ch := make(chan res, 1)
	go func() {
		defer func() {
			if r := recover(); r != nil {
				ch <- res{err: r}
			}
		}()
		ch <- res{s: Check(a, root)}
	}()
	select {
	case r := <-ch:
		if r.err != nil {
			t.Fatalf("Check panicked: %v", r.err)
		}
		return r.s
	case <-time.After(d):
		t.Fatalf("Check did not return within %v", d)
	}
	return Status{}
}

func TestCheckHostileRanges(t *testing.T) {
	root := t.TempDir()
	writeFile(t, root, "a.txt", "a\nb\nc\nd\n")
	cases := []struct {
		name       string
		start, end int
	}{
		{"huge end", 2, math.MaxInt},
		{"max start and end", math.MaxInt, math.MaxInt},
		{"huge start", 1 << 40, 1 << 40},
		{"huge start wide", 1 << 40, 1<<40 + 2},
		{"end past file", 1, 5},
		{"start past file", 9, 9},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			a := Anchor{Path: "a.txt", Start: c.start, End: c.end, Hash: "x"}
			if got := checkWithin(t, a, root, 2*time.Second); got.State != Changed {
				t.Fatalf("Check = %+v, want changed", got)
			}
		})
	}
}

func TestCheckStartPastEndFindsNearest(t *testing.T) {
	// The file shrank below the old Start: the nearest window is the last.
	root := t.TempDir()
	writeFile(t, root, "a.txt", "x\nb\nc\nx\nb\nc\n")
	a, err := MakeAnchor(root, "a.txt", 2, 3, "rk", "")
	if err != nil {
		t.Fatal(err)
	}
	a.Start, a.End = 1<<40, 1<<40+1
	if got := checkWithin(t, a, root, 2*time.Second); got != (Status{State: Moved, Start: 5, End: 6}) {
		t.Fatalf("Check = %+v", got)
	}
}

func TestCheckScanBudget(t *testing.T) {
	root := t.TempDir()
	var b strings.Builder
	for i := 0; i < 2000; i++ {
		fmt.Fprintf(&b, "line %d\n", i)
	}
	writeFile(t, root, "a.txt", b.String())
	a, err := MakeAnchor(root, "a.txt", 100, 199, "rk", "")
	if err != nil {
		t.Fatal(err)
	}
	// Move the lines far down; a generous budget finds them.
	moved := "pad\n" + strings.Repeat("y\n", 1500) + b.String()
	writeFile(t, root, "a.txt", moved)
	if got := Check(a, root); got != (Status{State: Moved, Start: 1601, End: 1700}) {
		t.Fatalf("Check = %+v", got)
	}
	// With a budget too small to reach them, the answer is Changed.
	old := scanBudget
	scanBudget = 10 << 10
	defer func() { scanBudget = old }()
	if got := Check(a, root); got.State != Changed {
		t.Fatalf("Check with small budget = %+v, want changed", got)
	}
}

func TestCheckLargeChangedIsBounded(t *testing.T) {
	if testing.Short() {
		t.Skip("large file")
	}
	root := t.TempDir()
	var b strings.Builder
	for i := 0; i < 200000; i++ {
		fmt.Fprintf(&b, "l%d\n", i)
	}
	writeFile(t, root, "a.txt", b.String())
	a, err := MakeAnchor(root, "a.txt", 1000, 3000, "rk", "")
	if err != nil {
		t.Fatal(err)
	}
	a.Hash = strings.Repeat("0", 64) // the lines changed
	start := time.Now()
	if got := checkWithin(t, a, root, 10*time.Second); got.State != Changed {
		t.Fatalf("Check = %+v", got)
	}
	if d := time.Since(start); d > slow*2*time.Second {
		t.Fatalf("Check took %v", d)
	}
}

func TestCheckMissingRootIsUnknown(t *testing.T) {
	root := t.TempDir()
	writeFile(t, root, "a.txt", "a\nb\n")
	a, err := MakeAnchor(root, "a.txt", 1, 1, "rk", "")
	if err != nil {
		t.Fatal(err)
	}
	if got := Check(a, filepath.Join(root, "nope")); got.State != Unknown {
		t.Fatalf("missing root: %+v", got)
	}
	file := filepath.Join(root, "a.txt")
	if got := Check(a, file); got.State != Unknown {
		t.Fatalf("root is a file: %+v", got)
	}
}

func TestMakeAnchorOnDiskCase(t *testing.T) {
	if runtime.GOOS != "windows" && runtime.GOOS != "darwin" {
		t.Skip("case-sensitive file system")
	}
	root := t.TempDir()
	writeFile(t, root, "Src/a.txt", "a\n")
	cases := []string{
		filepath.Join(root, "SRC", "A.TXT"),
		strings.ToUpper(root[:1]) + root[1:] + `\src\A.txt`,
		"src/A.TXT",
		"Src/a.txt",
	}
	for _, p := range cases {
		a, err := MakeAnchor(root, p, 1, 1, "rk", "")
		if err != nil {
			t.Fatalf("%q: %v", p, err)
		}
		if a.Path != "Src/a.txt" {
			t.Fatalf("%q: Path = %q, want Src/a.txt", p, a.Path)
		}
	}
}

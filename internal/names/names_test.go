package names

import (
	"strings"
	"testing"
)

func TestWordsAreDistinctAndPlain(t *testing.T) {
	seen := map[string]bool{}
	for _, w := range Words {
		if seen[w] {
			t.Errorf("%q twice", w)
		}
		seen[w] = true
		if w != strings.ToLower(w) || strings.ContainsAny(w, " -_") {
			t.Errorf("%q isn't a plain lowercase word", w)
		}
	}
}

func TestPickIsDeterministic(t *testing.T) {
	a := Pick("909ca932-7d4d-4226-a116-1e3079e08e2f", nil)
	if b := Pick("909ca932-7d4d-4226-a116-1e3079e08e2f", nil); a != b {
		t.Fatalf("same seed gave %q then %q", a, b)
	}
}

func TestPickProbesPastTakenNames(t *testing.T) {
	first := Pick("s1", nil)
	got := Pick("s1", func(n string) bool { return n == first })
	if got == first {
		t.Fatalf("picked the taken name %q", got)
	}
	// The next word along the list.
	i := 0
	for Words[i] != first {
		i++
	}
	if want := Words[(i+1)%len(Words)]; got != want {
		t.Fatalf("got %q, want the next word %q", got, want)
	}
}

func TestPickNumbersWhenEveryWordIsTaken(t *testing.T) {
	got := Pick("s1", func(n string) bool { return !strings.ContainsAny(n, "0123456789") })
	if want := Pick("s1", nil) + "2"; got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

func TestFold(t *testing.T) {
	if Fold("  Heron ") != "heron" {
		t.Fatal(Fold("  Heron "))
	}
}

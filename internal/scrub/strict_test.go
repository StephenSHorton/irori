package scrub

import "testing"

// Share to board's stricter pass (v0.5 §5.2, §11): absolute paths and home
// directories go; relative paths, URLs and prose stay.
func TestStrictDropsPaths(t *testing.T) {
	cases := []struct{ in, want string }{
		{`fix C:\Users\stephen\projects\irori\main.go now`, `fix [path] now`},
		{`see c:/work/x`, `see [path]`},
		{`under \\nas\share\docs`, `under [path]`},
		{`in ~/projects/mlg`, `in [path]`},
		{`in ~`, `in [path]`},
		{`at $HOME/.config/x`, `at [path]`},
		{`at %USERPROFILE%\AppData\Local\irori`, `at [path]`},
		{`/Users/stephen/projects/mlg/x.go breaks`, `[path] breaks`},
		{`log ("/home/me/logs/a.txt")`, `log ("[path]")`},
		{`path=/var/lib/irori/x`, `path=[path]`},
		// kept
		{`edit src/coord/impl.go and ./build/x`, `edit src/coord/impl.go and ./build/x`},
		{`https://github.com/StephenSHorton/irori/pull/12`, `https://github.com/StephenSHorton/irori/pull/12`},
		{`read and/or write 1/2 of it`, `read and/or write 1/2 of it`},
		{`use /help`, `use /help`},
		{`plain title`, `plain title`},
	}
	for _, c := range cases {
		if got := Strict(c.in); got != c.want {
			t.Errorf("Strict(%q) = %q, want %q", c.in, got, c.want)
		}
	}
	if !StrictDropped(`C:\x`) || StrictDropped("nothing here") {
		t.Fatal("StrictDropped disagrees with Strict")
	}
}

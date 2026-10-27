package coord

import (
	"errors"
	"testing"
)

func TestOverlaps(t *testing.T) {
	cases := []struct {
		a, b string
		want bool
	}{
		// literals
		{"a.go", "a.go", true},
		{"a.go", "b.go", false},
		{"src/a.go", "src/a.go", true},
		{"src/a.go", "lib/a.go", false},
		{"src/a.go", "src", false}, // a file vs its parent dir name only
		{"src", "src/a.go", false},
		{"./src/a.go", "src/a.go", true},
		{"Src/A.go", "src/a.go", true}, // case-insensitive, conservative
		// single star
		{"*", "a.go", true},
		{"*", "src/a.go", false}, // * stays inside one segment
		{"src/*", "src/a.go", true},
		{"src/*", "src/sub/a.go", false},
		{"*.go", "*.ts", false},
		{"*.go", "a.*", true},
		{"a*", "*b", true},
		{"a*b", "a*c", false},
		{"ab*", "a*", true},
		{"x*y*z", "*q*", true},
		{"*a*", "b", false},
		{"**.go", "x.go", true}, // ** inside a segment is two *
		// question mark
		{"a?.go", "ab.go", true},
		{"a?.go", "a.go", false},
		{"???", "ab", false},
		{"???", "*", true},
		{"?", "**", true},
		// classes
		{"[abc].go", "b.go", true},
		{"[abc].go", "d.go", false},
		{"[a-c].go", "[b-d].go", true},
		{"[0-3]", "[4-9]", false},
		{"[0-3]", "[3-9]", true},
		{"[^0].go", "0.go", false},
		{"[^a].go", "a.go", true}, // A.go matches [^a] and folds to a.go
		{"[^a].go", "b.go", true},
		{"[^0-9]", "[0-9]", false},
		{"[^0-4]", "[0-9]", true},
		{"[^x]", "[^y]", true},
		{"[a-z]", "[0-9]", false},
		{"[a-c]", "[X-Z]", true}, // two letter classes: assumed to meet (case)
		{"[A]", "a", true},
		{"?", "[0]", true},
		// globstar
		{"**", "anything/at/all.go", true},
		{"**", "x", true},
		{"src/**", "src/a/b/c.go", true},
		{"src/**", "src", true}, // ** matches zero segments
		{"src/**", "lib/a.go", false},
		{"src/", "src/a.go", true}, // trailing slash is src/**
		{"**/a.go", "a.go", true},
		{"**/a.go", "x/y/a.go", true},
		{"**/a.go", "x/y/b.go", false},
		{"src/**/test/*.go", "src/test/a.go", true},
		{"src/**/test/*.go", "src/x/y/test/a.go", true},
		{"src/**/test/*.go", "src/x/y/test/sub/a.go", false},
		{"src/**/test/*.go", "lib/**", false},
		{"a/**/z", "a/b/**", true},
		{"a/**/z", "**/y", false},
		{"**/x/**", "**/y/**", true}, // a/x/y/b matches both
		{"a/**/b/**/c", "a/b/c", true},
		{"a/**", "**/b", true},
		{"*/b", "a/*", true},
		{"*/b", "a/*/c", false},
		{"a/*/c", "a/**", true},
		// depth
		{"a/b", "a/b/c", false},
		{"a/*/*", "a/*", false},
		// unparseable leans true
		{"[", "a", true},
		{`a\b`, "x", true},
		{"/abs", "x", true},
	}
	for _, c := range cases {
		if got := Overlaps(c.a, c.b); got != c.want {
			t.Errorf("Overlaps(%q, %q) = %v, want %v", c.a, c.b, got, c.want)
		}
		if got := Overlaps(c.b, c.a); got != c.want {
			t.Errorf("Overlaps(%q, %q) = %v, want %v (swapped)", c.b, c.a, got, c.want)
		}
	}
}

func TestParseResource(t *testing.T) {
	ok := []struct{ in, want string }{
		{"path:remote:github.com/a/b:src/**", "path:remote:github.com/a/b:src/**"},
		{"path:root:4b825dc642cb6eb9a060e54bf8d69288fbee4904:src/*.go", "path:root:4b825dc642cb6eb9a060e54bf8d69288fbee4904:src/*.go"},
		{"path:remote:github.com/a/b:./docs/", "path:remote:github.com/a/b:docs/**"},
		{"path:root:abc:ab?c.txt", "path:root:abc:ab?c.txt"}, // ? stands for a ':'
		{"path:remote:::1/a/b:src", "path:remote:::1/a/b:src"},
		{"path:remote:host.com/a:b/c:src/", "path:remote:host.com/a:b/c:src/**"},
		{"path:remote:host.com/my org/repo:x", "path:remote:host.com/my org/repo:x"},
		{"path:remote:github.com/a/b:**", "path:remote:github.com/a/b:**"},
		{"  gpu/qwen ", "gpu/qwen"},
		{"build-cache", "build-cache"},
		{"db/main.v2", "db/main.v2"},
		{"port:8799", "port:8799"},
		{"port:1", "port:1"},
		{"port:65535", "port:65535"},
	}
	for _, c := range ok {
		got, err := NormalizeResource(c.in)
		if err != nil || got != c.want {
			t.Errorf("NormalizeResource(%q) = %q, %v; want %q", c.in, got, err, c.want)
		}
	}
	r, _ := ParseResource("path:root:abc:ab?c.txt")
	if r.Kind != ResourcePath || r.RepoKey != "root:abc" || r.Glob != "ab?c.txt" {
		t.Errorf("root parse = %+v", r)
	}
	r, _ = ParseResource("path:remote:github.com/a/b:src/**")
	if r.RepoKey != "remote:github.com/a/b" || r.Glob != "src/**" {
		t.Errorf("remote parse = %+v", r)
	}

	bad := []string{
		"",
		"path:",
		"path:remote:github.com/a/b",  // no glob
		"path:remote:github.com/a/b:", // empty glob
		"path:remote::src/**",         // empty repo value
		"path:other:x:src",            // unknown repoKey kind
		"path:github.com/a/b:src",     // bare remote
		`path:remote:github.com/a/b:src\a.go`,
		"path:remote:github.com/a/b:/etc/passwd",
		"path:root:abc:C:/Windows/x",
		"path:root:abc:./C:/x",
		"path:root:abc:.//etc/x",
		"path:root:abc:c:x",
		"path:root:abc:ab:c.txt", // a glob may not hold ':'
		"path:root:ABC:x",        // root is lowercase hex
		"path:root:xyz:x",
		"path:remote: host/a/b:x",
		"path:remote:host/a\x01b:x",
		"path:root:abc:a\nb",
		"path:root:abc:src/\x00x",
		"path:root:abc:../x",
		"path:root:abc:a/../x",
		"path:root:abc:a/./x",
		"path:root:abc:a//x",
		"path:root:abc:./",
		"path:root:abc:[",
		"path:root:abc:a/[/]",
		"port:0",
		"port:65536",
		"port:080",
		"port:x",
		"port:",
		"GPU/qwen",
		"gpu/",
		"/gpu",
		"-gpu",
		"gpu//qwen",
		"role:engine",
		"gpu qwen",
	}
	for _, in := range bad {
		if got, err := NormalizeResource(in); !errors.Is(err, ErrBadResource) {
			t.Errorf("NormalizeResource(%q) = %q, %v; want ErrBadResource", in, got, err)
		}
	}
}

func TestNewIDSortable(t *testing.T) {
	a := newID(fixedNow)
	b := newID(fixedNow.Add(1))
	c := newID(fixedNow.Add(2e6))
	if len(a) != 26 || a == b {
		t.Fatalf("ids %q %q", a, b)
	}
	if !(a[:10] < c[:10]) {
		t.Fatalf("not time-sorted: %q %q", a, c)
	}
}

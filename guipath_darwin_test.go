package main

import (
	"reflect"
	"strings"
	"testing"
)

func TestBetweenMarkers(t *testing.T) {
	out := "Last login: today\nwelcome banner\n" + pathMarker + "/opt/homebrew/bin:/usr/bin" + pathMarker + "\n"
	if got := between(out, pathMarker); got != "/opt/homebrew/bin:/usr/bin" {
		t.Fatalf("between = %q", got)
	}
	if got := between("no markers", pathMarker); got != "" {
		t.Fatalf("between without markers = %q", got)
	}
}

func TestMergePath(t *testing.T) {
	got := mergePath([]string{"/a", "", "/b", "/a"}, []string{"/usr/bin", "/b", "/bin"})
	if want := []string{"/a", "/b", "/usr/bin", "/bin"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("mergePath = %v, want %v", got, want)
	}
}

// The runner's login shell answers with a PATH that has /usr/bin in it.
func TestLoginShellPath(t *testing.T) {
	p := loginShellPath()
	if !strings.Contains(p, "/usr/bin") {
		t.Fatalf("login shell PATH = %q", p)
	}
}

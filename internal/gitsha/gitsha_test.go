package gitsha

import "testing"

func TestBlobKnownVectors(t *testing.T) {
	// `printf '' | git hash-object --stdin` and `printf 'hello\n' | git hash-object --stdin`.
	if got := Blob(nil); got != "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391" {
		t.Fatalf("empty blob = %s", got)
	}
	if got := Blob([]byte("hello\n")); got != "ce013625030ba8dba906f756967f9e9ca394464a" {
		t.Fatalf("hello blob = %s", got)
	}
}

func TestPair(t *testing.T) {
	raw, lf := Pair([]byte("hello\r\n"))
	if raw == lf || lf != "ce013625030ba8dba906f756967f9e9ca394464a" {
		t.Fatalf("raw %s lf %s", raw, lf)
	}
	if _, lf := Pair([]byte("hello\n")); lf != "" {
		t.Fatalf("LF-only data got lf %s", lf)
	}
}

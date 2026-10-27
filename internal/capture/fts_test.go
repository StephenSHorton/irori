package capture

import (
	"database/sql"
	"testing"

	_ "modernc.org/sqlite"
)

// TestFTS5Available keeps the search index's premise checked: the pure-Go
// SQLite driver has FTS5 with the porter tokenizer compiled in.
func TestFTS5Available(t *testing.T) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	db.SetMaxOpenConns(1)
	if _, err := db.Exec(`CREATE VIRTUAL TABLE d USING fts5(ref UNINDEXED, body, tokenize = 'porter unicode61')`); err != nil {
		t.Fatalf("fts5 missing: %v", err)
	}
	if _, err := db.Exec(`INSERT INTO d VALUES ('task:1', 'the lease was refused')`); err != nil {
		t.Fatal(err)
	}
	var ref, snip string
	if err := db.QueryRow(`SELECT ref, snippet(d, 1, '[', ']', '…', 8) FROM d WHERE d MATCH '"refuse"*' ORDER BY bm25(d)`).Scan(&ref, &snip); err != nil {
		t.Fatal(err)
	}
	if ref != "task:1" || snip != "the lease was [refused]" {
		t.Fatalf("got %q %q", ref, snip)
	}
}

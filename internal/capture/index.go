package capture

import (
	"database/sql"
	"fmt"
	"strings"
	"sync"
	"time"
	"unicode"

	"github.com/StephenSHorton/irori/internal/search"
	_ "modernc.org/sqlite"
)

// ftsIndex is the search index (search.go): one FTS5 table in a private
// in-memory database. Every ":memory:" connection is its own database, so
// the pool is pinned to one connection that is never closed while the
// index lives. Writes are upserts by ref; a Go map remembers each ref's
// rowid so an upsert deletes by rowid instead of scanning the ref column.
type ftsIndex struct {
	mu   sync.Mutex
	db   *sql.DB
	tx   *sql.Tx          // non-nil during a bulk load (replay)
	rows map[string]int64 // ref -> rowid
	next int64
}

func openIndex() (*ftsIndex, error) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		return nil, fmt.Errorf("capture: search index: %w", err)
	}
	db.SetMaxOpenConns(1)
	db.SetMaxIdleConns(1)
	db.SetConnMaxLifetime(0)
	db.SetConnMaxIdleTime(0)
	if _, err := db.Exec(`CREATE VIRTUAL TABLE docs USING fts5(
		ref UNINDEXED, kind UNINDEXED, at UNINDEXED, agent UNINDEXED, repo UNINDEXED, readers UNINDEXED,
		title, body, tokenize = 'porter unicode61')`); err != nil {
		db.Close()
		return nil, fmt.Errorf("capture: search index: %w", err)
	}
	return &ftsIndex{db: db, rows: map[string]int64{}}, nil
}

func (x *ftsIndex) close() error { return x.db.Close() }

// begin starts a bulk load: puts share one transaction until commit. A
// failed begin leaves puts autocommitting.
func (x *ftsIndex) begin() {
	x.mu.Lock()
	defer x.mu.Unlock()
	if x.tx == nil {
		x.tx, _ = x.db.Begin()
	}
}

func (x *ftsIndex) commit() error {
	x.mu.Lock()
	defer x.mu.Unlock()
	if x.tx == nil {
		return nil
	}
	err := x.tx.Commit()
	x.tx = nil
	return err
}

type execer interface {
	Exec(query string, args ...any) (sql.Result, error)
}

// put upserts one doc by its ref.
func (x *ftsIndex) put(d search.Doc) error {
	x.mu.Lock()
	defer x.mu.Unlock()
	var ex execer = x.db
	if x.tx != nil {
		ex = x.tx
	}
	rid, ok := x.rows[d.Ref]
	if ok {
		if _, err := ex.Exec(`DELETE FROM docs WHERE rowid = ?`, rid); err != nil {
			return err
		}
	} else {
		x.next++
		rid = x.next
		x.rows[d.Ref] = rid
	}
	_, err := ex.Exec(`INSERT INTO docs(rowid, ref, kind, at, agent, repo, readers, title, body) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		rid, d.Ref, string(d.Kind), unixNano(d.At), d.Agent, d.RepoKey, readersCol(d.Readers),
		clipRunes(oneLine(d.Title), search.MaxTitle), d.Body)
	return err
}

// readersSep wraps each reader key in the readers column. Keys never hold
// a newline (refs refuse whitespace), so instr on sep+key+sep matches
// whole keys only.
const readersSep = "\n"

// readersCol stores Readers as sep+a+sep+b+sep, or "" for every viewer.
func readersCol(rs []string) string {
	if len(rs) == 0 {
		return ""
	}
	return readersSep + strings.Join(rs, readersSep) + readersSep
}

// size is how many docs the index holds.
func (x *ftsIndex) size() int {
	x.mu.Lock()
	defer x.mu.Unlock()
	return len(x.rows)
}

// search runs q (Limit already clamped). The order is total (rowid breaks
// ties), so Offset pages without skipping or repeating a hit. Text that leaves no words after
// cleaning matches nothing.
func (x *ftsIndex) search(q search.Query) ([]search.Hit, error) {
	match, hasText := matchExpr(q.Text)
	if hasText && match == "" {
		return nil, nil
	}
	var where []string
	var args []any
	cols := `ref, kind, at, agent, repo, title, substr(body, 1, 2000)`
	order := `at DESC, rowid DESC`
	if hasText {
		cols = `ref, kind, at, agent, repo, title, snippet(docs, -1, '[', ']', '…', 24)`
		where = append(where, `docs MATCH ?`)
		args = append(args, match)
		order = `bm25(docs, 0, 0, 0, 0, 0, 4.0, 1.0), at DESC, rowid DESC`
	}
	if len(q.Kinds) > 0 {
		ph := make([]string, len(q.Kinds))
		for i, k := range q.Kinds {
			ph[i] = "?"
			args = append(args, string(k))
		}
		where = append(where, `kind IN (`+strings.Join(ph, ", ")+`)`)
	}
	if q.RepoKey != "" {
		where = append(where, `repo = ?`)
		args = append(args, q.RepoKey)
	}
	if q.Agent != "" {
		where = append(where, `agent = ?`)
		args = append(args, q.Agent)
	}
	if !q.Before.IsZero() {
		where = append(where, `at < ?`)
		args = append(args, unixNano(q.Before))
	}
	if q.Viewer != "" {
		where = append(where, `(readers = '' OR instr(readers, ?) > 0)`)
		args = append(args, readersSep+q.Viewer+readersSep)
	}
	stmt := `SELECT ` + cols + ` FROM docs`
	if len(where) > 0 {
		stmt += ` WHERE ` + strings.Join(where, ` AND `)
	}
	stmt += ` ORDER BY ` + order + ` LIMIT ? OFFSET ?`
	args = append(args, q.Limit, max(0, q.Offset))

	x.mu.Lock()
	defer x.mu.Unlock()
	rows, err := x.db.Query(stmt, args...)
	if err != nil {
		return nil, fmt.Errorf("capture: search: %w", err)
	}
	defer rows.Close()
	var out []search.Hit
	for rows.Next() {
		var h search.Hit
		var kind, snip string
		var at int64
		if err := rows.Scan(&h.Ref, &kind, &at, &h.Agent, &h.RepoKey, &h.Title, &snip); err != nil {
			return nil, fmt.Errorf("capture: search: %w", err)
		}
		h.Kind = search.Kind(kind)
		h.At = fromUnixNano(at)
		h.Snippet = clipRunes(oneLine(snip), search.MaxSnippet)
		out = append(out, h)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("capture: search: %w", err)
	}
	return out, nil
}

// matchExpr turns words into FTS5 syntax: each word quoted (quotes
// doubled), joined with AND, the last one a prefix. Words with no letter or
// digit are dropped (the tokenizer would drop them anyway, and an empty
// phrase is a syntax error). hasText is false for blank text.
func matchExpr(text string) (expr string, hasText bool) {
	words := strings.Fields(text)
	if len(words) == 0 {
		return "", false
	}
	var parts []string
	for _, w := range words {
		if !strings.ContainsFunc(w, func(r rune) bool { return unicode.IsLetter(r) || unicode.IsDigit(r) }) {
			continue
		}
		parts = append(parts, `"`+strings.ReplaceAll(w, `"`, `""`)+`"`)
	}
	if len(parts) == 0 {
		return "", true
	}
	parts[len(parts)-1] += "*"
	return strings.Join(parts, " AND "), true
}

// oneLine folds every run of whitespace (newlines included) into one space.
func oneLine(s string) string { return strings.Join(strings.Fields(s), " ") }

func unixNano(t time.Time) int64 {
	if t.IsZero() {
		return 0
	}
	return t.UnixNano()
}

func fromUnixNano(n int64) time.Time {
	if n == 0 {
		return time.Time{}
	}
	return time.Unix(0, n).UTC()
}

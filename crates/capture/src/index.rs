//! I:internal/capture/index.go: the search index, one FTS5 table in a
//! private in-memory SQLite database (capture.md §2.7). Nothing is on
//! disk: the index is rebuilt from the fold at [`crate::Capture::new`].
//!
//! Every `:memory:` connection is its own database, so the index holds one
//! connection for its whole life. Writes are upserts by ref: a map
//! remembers each ref's rowid, so an upsert deletes by rowid and inserts
//! with the same rowid instead of scanning the ref column.

use std::collections::HashMap;
use std::sync::Mutex;

use gocompat::time::GoTime;
use rusqlite::{Connection, params_from_iter, types::Value};
use search::{Doc, Hit, Kind, MAX_SNIPPET, MAX_TITLE, Query};

use crate::handle::clip_runes;

const CREATE: &str = "CREATE VIRTUAL TABLE docs USING fts5(
\t\tref UNINDEXED, kind UNINDEXED, at UNINDEXED, agent UNINDEXED, repo UNINDEXED, readers UNINDEXED,
\t\ttitle, body, tokenize = 'porter unicode61')";

struct State {
    db: Connection,
    /// Inside a bulk load (replay): puts share one transaction.
    in_tx: bool,
    /// ref -> rowid.
    rows: HashMap<String, i64>,
    next: i64,
}

/// The FTS5 index. Safe for concurrent use.
pub(crate) struct Index {
    st: Mutex<State>,
}

/// Wraps each reader key in the readers column. Keys never hold a newline
/// (refs refuse whitespace), so `instr` on sep+key+sep matches whole keys.
const READERS_SEP: &str = "\n";

fn readers_col(rs: &[String]) -> String {
    if rs.is_empty() {
        return String::new();
    }
    format!("{READERS_SEP}{}{READERS_SEP}", rs.join(READERS_SEP))
}

fn unix_nano(t: &GoTime) -> i64 {
    if t.is_zero() { 0 } else { t.unix_nano() }
}

fn from_unix_nano(n: i64) -> GoTime {
    if n == 0 {
        return GoTime::ZERO;
    }
    GoTime::unix(n.div_euclid(1_000_000_000), n.rem_euclid(1_000_000_000))
}

/// Go `strings.Fields`: the runs between `unicode.IsSpace` runs.
pub(crate) fn fields(s: &str) -> impl Iterator<Item = &str> {
    s.split(gocompat::text::is_space).filter(|w| !w.is_empty())
}

/// Every run of whitespace (newlines included) folded into one space.
pub(crate) fn one_line(s: &str) -> String {
    fields(s).collect::<Vec<_>>().join(" ")
}

/// Turns words into FTS5 syntax: each word quoted (quotes doubled), joined
/// with AND, the last one a prefix. Words with no letter or digit are
/// dropped (the tokenizer would drop them anyway, and an empty phrase is a
/// syntax error). The bool is false for blank text.
pub(crate) fn match_expr(text: &str) -> (String, bool) {
    let words: Vec<&str> = fields(text).collect();
    if words.is_empty() {
        return (String::new(), false);
    }
    let mut parts: Vec<String> = words
        .into_iter()
        .filter(|w| {
            w.chars()
                .any(|r| gocompat::text::is_letter(r) || gocompat::text::is_digit(r))
        })
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect();
    let Some(last) = parts.last_mut() else {
        return (String::new(), true);
    };
    last.push('*');
    (parts.join(" AND "), true)
}

impl Index {
    pub fn open() -> Result<Index, String> {
        let db = Connection::open_in_memory().map_err(|e| format!("capture: search index: {e}"))?;
        db.execute_batch(CREATE)
            .map_err(|e| format!("capture: search index: {e}"))?;
        Ok(Index {
            st: Mutex::new(State {
                db,
                in_tx: false,
                rows: HashMap::new(),
                next: 0,
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Starts a bulk load; a failed begin leaves puts autocommitting.
    pub fn begin(&self) {
        let mut s = self.lock();
        if !s.in_tx && s.db.execute_batch("BEGIN").is_ok() {
            s.in_tx = true;
        }
    }

    pub fn commit(&self) -> Result<(), String> {
        let mut s = self.lock();
        if !s.in_tx {
            return Ok(());
        }
        s.in_tx = false;
        s.db.execute_batch("COMMIT").map_err(|e| e.to_string())
    }

    /// Upserts one doc by its ref.
    pub fn put(&self, d: &Doc) -> Result<(), String> {
        let mut s = self.lock();
        let rid = match s.rows.get(&d.r#ref) {
            Some(&rid) => {
                s.db.execute("DELETE FROM docs WHERE rowid = ?", [rid])
                    .map_err(|e| e.to_string())?;
                rid
            }
            None => {
                s.next += 1;
                let rid = s.next;
                s.rows.insert(d.r#ref.clone(), rid);
                rid
            }
        };
        s.db.execute(
            "INSERT INTO docs(rowid, ref, kind, at, agent, repo, readers, title, body) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                rid,
                d.r#ref,
                d.kind.as_str(),
                unix_nano(&d.at),
                d.agent,
                d.repo_key,
                readers_col(&d.readers),
                clip_runes(&one_line(&d.title), MAX_TITLE),
                d.body,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// How many docs the index holds.
    pub fn size(&self) -> usize {
        self.lock().rows.len()
    }

    /// Runs `q` (limit already clamped). The order is total (rowid breaks
    /// ties), so offset pages without skipping or repeating a hit. Text that
    /// leaves no words after cleaning matches nothing.
    pub fn search(&self, q: &Query) -> Result<Vec<Hit>, String> {
        let (m, has_text) = match_expr(&q.text);
        if has_text && m.is_empty() {
            return Ok(Vec::new());
        }
        let mut wh: Vec<String> = Vec::new();
        let mut args: Vec<Value> = Vec::new();
        let mut cols = "ref, kind, at, agent, repo, title, substr(body, 1, 2000)";
        let mut order = "at DESC, rowid DESC";
        if has_text {
            cols = "ref, kind, at, agent, repo, title, snippet(docs, -1, '[', ']', '…', 24)";
            wh.push("docs MATCH ?".into());
            args.push(Value::Text(m));
            order = "bm25(docs, 0, 0, 0, 0, 0, 4.0, 1.0), at DESC, rowid DESC";
        }
        if !q.kinds.is_empty() {
            let ph = vec!["?"; q.kinds.len()].join(", ");
            for k in &q.kinds {
                args.push(Value::Text(k.as_str().into()));
            }
            wh.push(format!("kind IN ({ph})"));
        }
        if !q.repo_key.is_empty() {
            wh.push("repo = ?".into());
            args.push(Value::Text(q.repo_key.clone()));
        }
        if !q.agent.is_empty() {
            wh.push("agent = ?".into());
            args.push(Value::Text(q.agent.clone()));
        }
        if !q.before.is_zero() {
            wh.push("at < ?".into());
            args.push(Value::Integer(unix_nano(&q.before)));
        }
        if !q.viewer.is_empty() {
            wh.push("(readers = '' OR instr(readers, ?) > 0)".into());
            args.push(Value::Text(format!(
                "{READERS_SEP}{}{READERS_SEP}",
                q.viewer
            )));
        }
        let mut stmt = format!("SELECT {cols} FROM docs");
        if !wh.is_empty() {
            stmt += " WHERE ";
            stmt += &wh.join(" AND ");
        }
        stmt += " ORDER BY ";
        stmt += order;
        stmt += " LIMIT ? OFFSET ?";
        args.push(Value::Integer(q.limit));
        args.push(Value::Integer(q.offset.max(0)));

        let s = self.lock();
        let err = |e: rusqlite::Error| format!("capture: search: {e}");
        let mut st = s.db.prepare(&stmt).map_err(err)?;
        let rows = st
            .query_map(params_from_iter(args), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(err)?;
        let mut out = Vec::new();
        for row in rows {
            let (r, kind, at, agent, repo, title, snip) = row.map_err(err)?;
            let Some(kind) = Kind::from_name(&kind) else {
                continue;
            };
            out.push(Hit {
                r#ref: r,
                kind,
                at: from_unix_nano(at),
                title,
                snippet: clip_runes(&one_line(&snip), MAX_SNIPPET),
                agent,
                repo_key: repo,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TestFTS5Available: the bundled SQLite has FTS5 with the porter
    /// tokenizer compiled in.
    #[test]
    fn fts5_available() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE VIRTUAL TABLE d USING fts5(ref UNINDEXED, body, tokenize = 'porter unicode61')",
        )
        .expect("fts5 missing");
        db.execute(
            "INSERT INTO d VALUES ('task:1', 'the lease was refused')",
            [],
        )
        .unwrap();
        let (r, snip): (String, String) = db
            .query_row(
                "SELECT ref, snippet(d, 1, '[', ']', '…', 8) FROM d WHERE d MATCH '\"refuse\"*' ORDER BY bm25(d)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (r.as_str(), snip.as_str()),
            ("task:1", "the lease was [refused]")
        );
    }

    /// TestMatchExpr.
    #[test]
    fn match_expr_cases() {
        let cases: &[(&str, &str, bool)] = &[
            ("", "", false),
            ("   \t\n", "", false),
            ("lease", r#""lease"*"#, true),
            ("lease refused", r#""lease" AND "refused"*"#, true),
            (r#"say "hi""#, r#""say" AND """hi"""*"#, true),
            ("a-b OR c", r#""a-b" AND "OR" AND "c"*"#, true),
            ("NEAR( x", r#""NEAR(" AND "x"*"#, true),
            ("- * ( )", "", true),
            ("日本 語", r#""日本" AND "語"*"#, true),
        ];
        for &(text, want, has) in cases {
            assert_eq!(match_expr(text), (want.to_string(), has), "{text:?}");
        }
    }

    #[test]
    fn unix_nano_round_trip() {
        let t = GoTime::unix(1_700_000_000, 123_456_789);
        assert_eq!(from_unix_nano(unix_nano(&t)), t);
        assert!(from_unix_nano(0).is_zero());
        assert_eq!(unix_nano(&GoTime::ZERO), 0);
    }
}

//! I:internal/capture/tailer.go: finding transcripts, the backfill rules,
//! and reading new complete lines into the parser.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::time::SystemTime;

use gocompat::time::GoTime;
use sys::cancel::Cancel;

use crate::cursors::CursorFile;
use crate::gopath;
use crate::rows::open_key;
use crate::service::{Change, ChangeKind};
use crate::service::{Inner, RunState};
use crate::transcript::{self, Parser, Source};
use crate::{AgentCard, gotime};

/// Bytes a file pass reads between cursor saves.
pub const SAVE_EVERY: i64 = 4 << 20;
/// The line reader's buffer.
pub const READ_BUF: usize = 1 << 20;
/// A longer line is consumed, never decoded.
pub const MAX_LINE: usize = 64 << 20;

const COMPACT_BOUNDARY: &[u8] = br#""subtype":"compact_boundary""#;

/// One transcript being tailed; owned by the run loop.
pub(crate) struct TFile {
    pub path: String,
    pub src: Source,
    pub key: String,
    pub parser: Parser,
    /// Bytes fed to the parser.
    pub offset: i64,
    /// The offset of the line that opened the open turn, -1 when none.
    pub open_start: i64,
    /// The cursor last handed to the cursor file.
    pub saved: i64,
    pub size: i64,
    pub last_grow: GoTime,
    /// Postpones the next quiet check of a turn kept open by a busy card.
    pub quiet_at: GoTime,
}

impl TFile {
    /// Where a restart resumes: the open turn's first line (the turn is
    /// rebuilt whole; rows past it dedupe by id), else the end of what was
    /// read.
    pub fn cursor(&self) -> i64 {
        if self.open_start >= 0 {
            self.open_start
        } else {
            self.offset
        }
    }
}

/// A transcript found on disk.
#[derive(Clone)]
pub(crate) struct Found {
    pub path: String,
    pub src: Source,
    pub size: i64,
    pub modt: GoTime,
}

pub(crate) fn mtime(m: &std::fs::Metadata) -> GoTime {
    gotime::local(GoTime::from_system(
        m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
    ))
}

/// Reads complete lines from `r` (whose first byte is at offset `base`) and
/// calls `f` with each line's offset and bytes (newline included). Stops
/// when `f` returns false or at a trailing partial line; returns the bytes
/// consumed. Lines over [`MAX_LINE`] are consumed without a call.
pub(crate) fn each_line(
    r: impl Read,
    base: i64,
    mut f: impl FnMut(i64, &[u8]) -> bool,
) -> std::io::Result<i64> {
    let mut br = BufReader::with_capacity(READ_BUF, r);
    let mut consumed: i64 = 0;
    let mut line: Vec<u8> = Vec::new();
    let mut pending: usize = 0;
    let mut over = false;
    loop {
        let buf = match br.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if buf.is_empty() {
            return Ok(consumed);
        }
        match buf.iter().position(|&c| c == b'\n') {
            Some(i) => {
                let chunk = &buf[..=i];
                pending += chunk.len();
                if !over {
                    line.extend_from_slice(chunk);
                }
                let start = base + consumed;
                consumed += pending as i64;
                let deliver = !over;
                pending = 0;
                over = false;
                br.consume(i + 1);
                if deliver {
                    let l = std::mem::take(&mut line);
                    if !f(start, &l) {
                        return Ok(consumed);
                    }
                } else {
                    line.clear();
                }
            }
            None => {
                let n = buf.len();
                pending += n;
                if !over {
                    if pending > MAX_LINE {
                        over = true;
                        line = Vec::new();
                    } else {
                        line.extend_from_slice(buf);
                    }
                }
                br.consume(n);
            }
        }
    }
}

/// Scans the last `max` bytes of a file for its last compact_boundary row
/// and returns that row's offset, else the first line start in that window.
pub(crate) fn boundary_start(path: &str, size: i64, max: i64) -> std::io::Result<i64> {
    let mut fh = std::fs::File::open(path)?;
    let low = (size - max).max(0);
    let mut prev_nl = true;
    if low > 0 {
        let mut b = [0u8; 1];
        fh.seek(SeekFrom::Start((low - 1) as u64))?;
        prev_nl = fh.read_exact(&mut b).is_ok() && b[0] == b'\n';
    }
    fh.seek(SeekFrom::Start(low as u64))?;
    let (mut first, mut best) = (-1i64, -1i64);
    each_line(fh.take((size - low) as u64), low, |start, line| {
        if low > 0 && start == low && !prev_nl {
            // The window starts mid-line.
            return true;
        }
        if first < 0 {
            first = start;
        }
        if line
            .windows(COMPACT_BOUNDARY.len())
            .any(|w| w == COMPACT_BOUNDARY)
        {
            best = start;
        }
        true
    })?;
    Ok(if best >= 0 {
        best
    } else if first >= 0 {
        first
    } else {
        size
    })
}

impl Inner {
    pub(crate) fn load_cursors(&self, rs: &mut RunState) {
        if rs.cursors.is_some() {
            return;
        }
        let (cf, err) = CursorFile::load(&self.opts.data_dir);
        if let Some(e) = err {
            self.set_err(e);
        }
        rs.cursors = Some(cf);
    }

    pub(crate) fn save_cursors(&self, rs: &mut RunState) {
        if let Some(cf) = rs.cursors.as_mut()
            && let Err(e) = cf.save(&self.opts.data_dir)
        {
            self.set_err(e);
        }
    }

    pub(crate) fn projects_dir(&self) -> String {
        gopath::join(&[&self.opts.claude_dir, "projects"])
    }

    /// projects/<slug>/<sid>.jsonl and projects/<slug>/<sid>/subagents/**/agent-<id>.jsonl.
    pub(crate) fn list_files(&self) -> Vec<Found> {
        let projects = self.projects_dir();
        let Ok(slugs) = read_dir_sorted(&projects) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut add = |p: String, m: &std::fs::Metadata| {
            if let Some(src) = transcript::source_for(&projects, &p) {
                out.push(Found {
                    path: p,
                    src,
                    size: m.len() as i64,
                    modt: mtime(m),
                });
            }
        };
        for (name, is_dir) in slugs {
            if !is_dir {
                continue;
            }
            let dir = gopath::join(&[&projects, &name]);
            let Ok(ents) = read_dir_sorted(&dir) else {
                continue;
            };
            for (ename, edir) in ents {
                let p = gopath::join(&[&dir, &ename]);
                if !edir {
                    if ename.ends_with(".jsonl")
                        && let Ok(m) = std::fs::metadata(&p)
                    {
                        add(p, &m);
                    }
                    continue;
                }
                let sub = gopath::join(&[&p, "subagents"]);
                if !crate::roots::dir_exists(&sub) {
                    continue;
                }
                walk(&sub, &mut |q, m| {
                    if q.ends_with(".jsonl") {
                        add(q.to_string(), m);
                    }
                });
            }
        }
        out
    }

    /// Reads every transcript's new lines once.
    pub(crate) fn tail_pass(&self, rs: &mut RunState, cancel: &Cancel) {
        self.load_cursors(rs);
        let live = self.live_cards();
        let mut files = self.list_files();
        // Live sessions first, then newest first.
        files.sort_by(|a, b| {
            let la = live.contains_key(&a.src.session_id);
            let lb = live.contains_key(&b.src.session_id);
            lb.cmp(&la).then_with(|| gotime::cmp(&b.modt, &a.modt))
        });
        for fd in files {
            if cancel.is_canceled() {
                return;
            }
            let card = live.get(&fd.src.session_id).cloned();
            self.tail_file(rs, cancel, fd, card);
        }
        self.end_tail(rs, cancel, true);
    }

    /// Reads the new lines of the named transcripts only; anything else is
    /// ignored.
    pub(crate) fn tail_paths(&self, rs: &mut RunState, cancel: &Cancel, paths: &[String]) {
        self.load_cursors(rs);
        let projects = self.projects_dir();
        let live = self.live_cards();
        for p in paths {
            if cancel.is_canceled() {
                return;
            }
            let Some(src) = transcript::source_for(&projects, p) else {
                continue;
            };
            let Ok(m) = std::fs::metadata(p) else {
                continue;
            };
            if m.is_dir() {
                continue;
            }
            let card = live.get(&src.session_id).cloned();
            let fd = Found {
                path: p.clone(),
                src,
                size: m.len() as i64,
                modt: mtime(&m),
            };
            self.tail_file(rs, cancel, fd, card);
        }
        self.end_tail(rs, cancel, false);
    }

    /// Saves cursors, relinks and refreshes Status after a tail.
    pub(crate) fn end_tail(&self, rs: &mut RunState, cancel: &Cancel, full: bool) {
        self.save_cursors(rs);
        self.relink(rs, cancel);
        let backlog: i64 = rs
            .files
            .values()
            .filter(|f| f.size > f.offset)
            .map(|f| f.size - f.offset)
            .sum();
        let now = self.now();
        let first_done = full && rs.first_pass;
        if first_done {
            rs.first_pass = false;
        }
        let mut f = self.write();
        f.backlog = backlog;
        f.files = rs.files.len() as i64;
        f.last_scan = now;
        if first_done {
            f.backfilling = false;
        }
    }

    /// Starts tailing a file, choosing where to begin.
    fn track(&self, rs: &mut RunState, fd: &Found, is_live: bool) {
        let key = format!("{}/{}", self.opts.device_id, fd.src.session_id);
        let mut f = TFile {
            path: fd.path.clone(),
            src: fd.src.clone(),
            key,
            parser: Parser::new(fd.src.clone()),
            offset: 0,
            open_start: -1,
            saved: 0,
            size: 0,
            last_grow: fd.modt,
            quiet_at: GoTime::ZERO,
        };
        let cf = rs.cursors.as_mut().expect("cursors loaded");
        if let Some(&off) = cf.files.get(&fd.path) {
            f.offset = if off > fd.size { 0 } else { off };
        } else {
            f.offset = self.start_offset(fd, is_live);
            cf.set_file(&fd.path, f.offset);
        }
        f.saved = f.offset;
        rs.files.insert(fd.path.clone(), f);
    }

    /// The backfill rules for a file seen for the first time.
    fn start_offset(&self, fd: &Found, is_live: bool) -> i64 {
        let max = self.opts.backfill_max_bytes;
        let age = gotime::sub_nanos(&self.now(), &fd.modt);
        if fd.size <= max && age <= self.opts.backfill_window.as_nanos() as i128 {
            return 0;
        }
        if is_live
            && fd.size > max
            && let Ok(off) = boundary_start(&fd.path, fd.size, max)
        {
            return off;
        }
        fd.size
    }

    /// Reads one file's new lines into rows.
    pub(crate) fn tail_file(
        &self,
        rs: &mut RunState,
        cancel: &Cancel,
        fd: Found,
        card: Option<AgentCard>,
    ) {
        if !rs.files.contains_key(&fd.path) {
            self.track(rs, &fd, card.is_some());
        }
        let mut f = rs.files.remove(&fd.path).expect("tracked");
        if fd.size < f.offset {
            // Shrunk: start over.
            f.offset = 0;
            f.open_start = -1;
            f.parser = Parser::new(f.src.clone());
        }
        f.size = fd.size;
        let mut grew = false;
        if fd.size > f.offset {
            grew = self.read_file(rs, cancel, &mut f, fd.size);
        }
        let now = self.now();
        if grew {
            f.last_grow = now;
        }
        let busy = card.as_ref().is_some_and(|c| c.status == "busy");
        if f.parser.is_open()
            && gotime::sub_nanos(&now, &f.last_grow) >= self.opts.quiet_after.as_nanos() as i128
            && !busy
        {
            let mut at = f.parser.last_activity();
            if at.is_zero() {
                at = now;
            }
            let items = f.parser.close(at, "quiet");
            if let Err(e) = self.handle(rs, cancel, &f, &items) {
                self.set_err(e);
            }
            f.open_start = -1;
        }
        let cur = f.cursor();
        rs.cursors
            .as_mut()
            .expect("cursors loaded")
            .set_file(&f.path, cur);
        f.saved = cur;

        let ok = open_key(&f.key, &f.src.agent_id);
        let open = f.parser.open();
        let has_open = open.is_some();
        {
            let mut fold = self.write();
            match open {
                Some(t) => {
                    fold.open.insert(ok, t);
                }
                None => {
                    fold.open.remove(&ok);
                }
            }
        }
        let key = f.key.clone();
        rs.files.insert(f.path.clone(), f);
        if grew && has_open {
            self.notify(Change {
                kind: ChangeKind::Open,
                key,
                repo_key: String::new(),
            });
        }
    }

    /// Feeds [offset, size) to the parser up to the last newline; reports
    /// whether anything was read. On a failed append the parser is reset to
    /// the last saved cursor, so the next pass reads those lines again.
    fn read_file(&self, rs: &mut RunState, cancel: &Cancel, f: &mut TFile, size: i64) -> bool {
        let Ok(mut fh) = std::fs::File::open(&f.path) else {
            return false;
        };
        if fh.seek(SeekFrom::Start(f.offset as u64)).is_err() {
            return false;
        }
        let start = f.offset;
        let mut failed: Option<String> = None;
        let mut since_save: i64 = 0;
        let res = each_line(fh.take((size - f.offset) as u64), f.offset, |at, line| {
            if cancel.is_canceled() {
                failed = Some("context canceled".into());
                return false;
            }
            let end = at + line.len() as i64;
            if !transcript::skip_line(line) {
                let had_open = f.open_start >= 0;
                let items = f.parser.line(line);
                let closed = items
                    .iter()
                    .any(|it| matches!(it, transcript::Item::Turn(_)));
                if !had_open || closed {
                    f.open_start = if f.parser.is_open() { at } else { -1 };
                }
                if let Err(e) = self.handle(rs, cancel, f, &items) {
                    failed = Some(e);
                    return false;
                }
            }
            since_save += end - f.offset;
            f.offset = end;
            if since_save >= SAVE_EVERY {
                since_save = 0;
                let cur = f.cursor();
                rs.cursors
                    .as_mut()
                    .expect("cursors loaded")
                    .set_file(&f.path, cur);
                f.saved = cur;
                self.save_cursors(rs);
            }
            true
        });
        if let Err(e) = res
            && failed.is_none()
        {
            failed = Some(e.to_string());
        }
        if let Some(e) = failed {
            if !cancel.is_canceled() {
                self.set_err(&e);
                eprintln!("capture: {}: {e}", gopath::base(&f.path));
            }
            // Re-read from the last saved cursor next time; ids dedupe.
            f.parser = Parser::new(f.src.clone());
            f.offset = f.saved;
            f.open_start = -1;
            return false;
        }
        f.offset > start
    }
}

/// A directory's entries as (name, is_dir), sorted by name (Go `os.ReadDir`).
pub(crate) fn read_dir_sorted(dir: &str) -> std::io::Result<Vec<(String, bool)>> {
    let mut out: Vec<(String, bool)> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| {
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            (e.file_name().to_string_lossy().into_owned(), is_dir)
        })
        .collect();
    out.sort();
    Ok(out)
}

/// Go `filepath.WalkDir` over regular entries, lexical order; errors skip.
fn walk(dir: &str, f: &mut dyn FnMut(&str, &std::fs::Metadata)) {
    let Ok(ents) = read_dir_sorted(dir) else {
        return;
    };
    for (name, is_dir) in ents {
        let p = gopath::join(&[dir, &name]);
        if is_dir {
            walk(&p, f);
        } else if let Ok(m) = std::fs::metadata(&p) {
            f(&p, &m);
        }
    }
}

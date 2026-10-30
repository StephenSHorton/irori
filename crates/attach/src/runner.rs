//! `irori attach`'s loop (I:internal/attach/runner/runner.go; surface.md
//! §3 "CLI rules"): dial the running Core through the lockfile, open
//! `GET /v1/attach?session=<id>&mod=<v>` and copy each event line to stdout
//! unchanged, reconnecting with backoff until canceled or superseded.
//!
//! stdout is the mod's channel: nothing but event lines is ever written to
//! it (keepalives and lines that aren't JSON are dropped). Diagnostics go
//! to [`Options::log`] (stderr in the binary); none carries the token.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use http_body_util::BodyExt;
use tokio_util::sync::CancellationToken;

use crate::client::{self, Error};
use irori_api::attach::{self as p, Event};

/// The one line an outage writes (Go `attach.NotRunning`).
pub const NOT_RUNNING: &str = "Irori isn't running, so messages can't reach this session. It attaches by itself once Irori starts.";

pub const MIN_BACKOFF: Duration = Duration::from_secs(1);
pub const MAX_BACKOFF: Duration = Duration::from_secs(16);

/// Caps one event line (a wake prompt is at most ~16 KB of text, escaped).
pub const MAX_LINE: usize = 1 << 20;

type LogFn = Box<dyn Fn(&str) + Send + Sync>;

/// Configures [`run`].
pub struct Options {
    /// The Claude Code session id.
    pub session: String,
    /// Informational (logs).
    pub cwd: String,
    pub mod_version: String,
    /// Where the lockfile is.
    pub data_dir: PathBuf,
    /// Event lines only.
    pub stdout: Box<dyn Write + Send>,
    pub log: LogFn,
    /// Ends the run (the parent died, a signal, a test).
    pub cancel: CancellationToken,
    /// Zero means [`MIN_BACKOFF`] / [`MAX_BACKOFF`].
    pub min_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            session: String::new(),
            cwd: String::new(),
            mod_version: String::new(),
            data_dir: PathBuf::new(),
            stdout: Box::new(std::io::sink()),
            log: Box::new(|_| {}),
            cancel: CancellationToken::new(),
            min_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
        }
    }
}

/// Why one attempt ended.
enum End {
    /// The stream ended (EOF, or a read error after hello).
    Ended,
    Superseded,
    Canceled,
    /// stdout can't be written: [`run`] fails.
    Stdout(std::io::Error),
    Failed(Error),
}

/// Attaches until canceled (`Ok`), superseded (`Ok`) or stdout can't be
/// written (an error). Go `runner.Run`.
pub async fn run(mut o: Options) -> Result<(), String> {
    if o.session.is_empty() {
        return Err("attach: --session is required".into());
    }
    if o.min_backoff.is_zero() {
        o.min_backoff = MIN_BACKOFF;
    }
    if o.max_backoff.is_zero() {
        o.max_backoff = MAX_BACKOFF;
    }
    let mut backoff = o.min_backoff;
    let mut outage = false; // the "Irori isn't running" line went out
    loop {
        let (connected, end) = attach_once(&mut o).await;
        if connected {
            backoff = o.min_backoff;
            outage = false;
        }
        match end {
            End::Superseded => {
                (o.log)(&format!(
                    "attach: superseded by a newer attach for {}",
                    o.session
                ));
                return Ok(());
            }
            End::Canceled => return Ok(()),
            End::Stdout(e) => return Err(format!("attach: stdout closed: {e}")),
            _ if o.cancel.is_cancelled() => return Ok(()),
            End::Ended => (o.log)("attach: stream ended; reconnecting"),
            End::Failed(Error::Status(se)) if se.code == 404 => (o.log)(&format!(
                "attach: session {} not on the roster yet; retrying",
                o.session
            )),
            End::Failed(Error::Status(se)) => (o.log)(&format!("attach: {se}")),
            End::Failed(e @ Error::Down(_)) => {
                (o.log)(&format!("attach: {e}"));
                if !outage {
                    outage = true;
                    let ev = Event {
                        message: NOT_RUNNING.into(),
                        ..Event::of(p::TYPE_ERROR)
                    };
                    // An Event holds only strings, ints and bools: marshal
                    // cannot fail.
                    let line = gocompat::json::marshal(&ev).expect("attach event marshals");
                    if let Err(e) = write_line(&mut o.stdout, &line) {
                        return Err(format!("attach: stdout closed: {e}"));
                    }
                }
            }
        }
        tokio::select! {
            _ = o.cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(o.max_backoff);
    }
}

fn write_line(w: &mut Box<dyn Write + Send>, line: &[u8]) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(line.len() + 1);
    buf.extend_from_slice(line);
    buf.push(b'\n');
    w.write_all(&buf)?;
    w.flush()
}

/// Dials and copies one stream. The bool says a hello arrived (an outage,
/// if any, is over).
async fn attach_once(o: &mut Options) -> (bool, End) {
    let cancel = o.cancel.clone();
    tokio::select! {
        _ = cancel.cancelled() => (false, End::Canceled),
        r = attach_inner(o) => r,
    }
}

async fn attach_inner(o: &mut Options) -> (bool, End) {
    let c = match client::dial(&o.data_dir).await {
        Ok(c) => c,
        Err(e) => return (false, End::Failed(e)),
    };
    let mut pairs = vec![("session", o.session.as_str())];
    if !o.mod_version.is_empty() {
        pairs.push(("mod", o.mod_version.as_str()));
    }
    let mut body = match c.open_stream(&client::encode_query(&pairs)).await {
        Ok(b) => b,
        Err(Error::Down(s)) => {
            return (
                false,
                End::Failed(Error::Down(format!("{s} (open stream)"))),
            );
        }
        Err(e) => return (false, End::Failed(e)),
    };
    (o.log)(&format!(
        "attach: connected to irori on port {} (session {}, cwd {})",
        c.port(),
        o.session,
        gocompat::quote::quote(&o.cwd)
    ));
    let mut connected = false;
    let mut lines = Lines::default();
    loop {
        let frame = match body.frame().await {
            None => return (connected, End::Ended),
            Some(Err(e)) => {
                if connected {
                    (o.log)(&format!("attach: stream: {e}"));
                    return (connected, End::Ended);
                }
                return (connected, End::Failed(Error::Down(e.to_string())));
            }
            Some(Ok(f)) => f,
        };
        let Ok(data) = frame.into_data() else {
            continue;
        };
        for line in lines.push(&data) {
            if line.is_empty() {
                continue; // a keepalive
            }
            let Some(t) = event_type(&line) else {
                (o.log)(&format!(
                    "attach: skipping a line that isn't JSON ({} bytes)",
                    line.len()
                ));
                continue;
            };
            if let Err(e) = write_line(&mut o.stdout, &line) {
                return (connected, End::Stdout(e));
            }
            match t.as_str() {
                p::TYPE_HELLO => connected = true,
                p::TYPE_SUPERSEDED => return (connected, End::Superseded),
                _ => {}
            }
        }
    }
}

/// The line's `type` as Go reads it (`json.Unmarshal` into
/// `struct{Type string}`); `None` when that fails.
fn event_type(line: &[u8]) -> Option<String> {
    use gocompat::json::lenient::{self, Schema};
    let schema = Schema::object(vec![("type", Schema::String)]);
    lenient::unmarshal(line, &schema)
        .ok()
        .map(|o| o.get_str("type").to_string())
}

/// Splits a byte stream into lines (Go `readLine` over a bufio.Reader):
/// each without its `\n` and trimmed of white space; a line longer than
/// [`MAX_LINE`] is dropped whole.
#[derive(Default)]
struct Lines {
    buf: Vec<u8>,
    /// The current line overflowed: drop through its newline.
    skipping: bool,
}

impl Lines {
    fn push(&mut self, mut data: &[u8]) -> Vec<Vec<u8>> {
        let mut out = vec![];
        while !data.is_empty() {
            let (chunk, rest, nl) = match data.iter().position(|&c| c == b'\n') {
                Some(i) => (&data[..=i], &data[i + 1..], true),
                None => (data, &data[data.len()..], false),
            };
            data = rest;
            if !self.skipping && self.buf.len() + chunk.len() > MAX_LINE {
                self.skipping = true;
                self.buf.clear();
            }
            if self.skipping {
                if nl {
                    self.skipping = false;
                }
                continue;
            }
            self.buf.extend_from_slice(chunk);
            if nl {
                out.push(trim_space(&self.buf).to_vec());
                self.buf.clear();
            }
        }
        out
    }
}

/// Go `bytes.TrimSpace` for the ASCII white space a line can carry.
fn trim_space(b: &[u8]) -> &[u8] {
    let is = |c: &u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c');
    let start = b.iter().position(|c| !is(c)).unwrap_or(b.len());
    let end = b.iter().rposition(|c| !is(c)).map_or(start, |i| i + 1);
    &b[start..end]
}

#[cfg(test)]
mod tests {
    use super::{Lines, MAX_LINE, event_type};

    #[test]
    fn lines_split_trim_and_drop_oversized() {
        let mut l = Lines::default();
        assert!(l.push(b"{\"type\":\"he").is_empty());
        assert_eq!(
            l.push(b"llo\"}\r\n\n x \n"),
            vec![b"{\"type\":\"hello\"}".to_vec(), vec![], b"x".to_vec()]
        );
        let big = vec![b'a'; MAX_LINE + 1];
        assert!(l.push(&big).is_empty());
        assert_eq!(l.push(b"tail\nnext\n"), vec![b"next".to_vec()]);
    }

    #[test]
    fn event_types_as_go_reads_them() {
        assert_eq!(
            event_type(b"{\"type\":\"deliver\"}").as_deref(),
            Some("deliver")
        );
        assert_eq!(event_type(b"{\"TYPE\":\"x\"}").as_deref(), Some("x"));
        assert_eq!(event_type(b"null").as_deref(), Some(""));
        assert_eq!(event_type(b"not json"), None);
        assert_eq!(event_type(b"[1]"), None);
        assert_eq!(event_type(b"{\"type\":1}"), None);
    }
}

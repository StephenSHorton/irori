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

/// A stream that had a hello ended (card #111): its attach secret is dead,
/// so the mod must stop using it until the next hello.
pub const DROPPED: &str = "Irori's connection to this session dropped; reattaching.";

/// The session stayed off Irori's roster past [`Options::roster_grace`]
/// (Go has no such line; card #111): said once, so the mod doesn't sit on
/// "connecting" with no reason.
pub fn not_on_roster(session: &str) -> String {
    format!(
        "Irori doesn't list session {session} as running on this device, so it can't attach yet. \
It keeps trying; if this lasts, restart this Claude Code session."
    )
}

/// Bounds the wait for `/v1/attach`'s answer and then for its hello: a Core
/// that takes the connection and says nothing is as good as down.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
/// No byte for this long on an attached stream (the Core sends a keepalive
/// every 15 s) ends it, so a stream the Core lost without closing it
/// doesn't hold the session forever.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
/// How long 404s ("not on the roster yet") go unreported.
pub const ROSTER_GRACE: Duration = Duration::from_secs(20);

/// How often a standby attach asks whether the session came free.
pub const STANDBY_POLL: Duration = Duration::from_secs(2);

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
    /// Never take the session from a live attach: wait, quietly, until it
    /// is free (GitHub #57). What a copy that was superseded runs.
    pub standby: bool,
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
    /// Zero means [`OPEN_TIMEOUT`] / [`IDLE_TIMEOUT`] / [`ROSTER_GRACE`].
    pub open_timeout: Duration,
    pub idle_timeout: Duration,
    pub roster_grace: Duration,
    /// Zero means [`STANDBY_POLL`].
    pub standby_poll: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            session: String::new(),
            cwd: String::new(),
            mod_version: String::new(),
            standby: false,
            data_dir: PathBuf::new(),
            stdout: Box::new(std::io::sink()),
            log: Box::new(|_| {}),
            cancel: CancellationToken::new(),
            min_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
            open_timeout: Duration::ZERO,
            idle_timeout: Duration::ZERO,
            roster_grace: Duration::ZERO,
            standby_poll: Duration::ZERO,
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
    if o.open_timeout.is_zero() {
        o.open_timeout = OPEN_TIMEOUT;
    }
    if o.idle_timeout.is_zero() {
        o.idle_timeout = IDLE_TIMEOUT;
    }
    if o.roster_grace.is_zero() {
        o.roster_grace = ROSTER_GRACE;
    }
    if o.standby_poll.is_zero() {
        o.standby_poll = STANDBY_POLL;
    }
    let mut backoff = o.min_backoff;
    let mut outage = false; // the "Irori isn't running" line went out
    // Since when the Core has answered 404, and whether that was said.
    let mut off_roster: Option<(std::time::Instant, bool)> = None;
    // The `standby` line went out for this wait.
    let mut waiting = false;
    loop {
        let (connected, end) = attach_once(&mut o).await;
        if !matches!(&end, End::Failed(Error::Status(se)) if se.code == 409) {
            waiting = false;
        }
        if connected {
            backoff = o.min_backoff;
            outage = false;
            off_roster = None;
            // Whatever ended the stream, its secret is dead now.
            if !matches!(end, End::Superseded | End::Canceled | End::Stdout(_))
                && !o.cancel.is_cancelled()
                && let Err(e) = write_error(&mut o.stdout, DROPPED)
            {
                return Err(format!("attach: stdout closed: {e}"));
            }
        }
        if !matches!(&end, End::Failed(Error::Status(se)) if se.code == 404) {
            off_roster = None;
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
            // Waiting its turn is no outage and takes no backoff. Said once
            // per wait: whatever the mod last heard (an outage, a dropped
            // stream) is no longer why this session has no attach.
            End::Failed(Error::Status(se)) if se.code == 409 => {
                outage = false;
                backoff = o.min_backoff;
                if !waiting {
                    waiting = true;
                    let line = gocompat::json::marshal(&Event::of(p::TYPE_STANDBY))
                        .expect("attach event marshals");
                    if let Err(e) = write_line(&mut o.stdout, &line) {
                        return Err(format!("attach: stdout closed: {e}"));
                    }
                }
                tokio::select! {
                    _ = o.cancel.cancelled() => return Ok(()),
                    _ = tokio::time::sleep(o.standby_poll) => {}
                }
                continue;
            }
            End::Ended => (o.log)("attach: stream ended; reconnecting"),
            End::Failed(Error::Status(se)) if se.code == 404 => {
                (o.log)(&format!(
                    "attach: session {} not on the roster yet; retrying",
                    o.session
                ));
                let (since, said) = off_roster.get_or_insert((std::time::Instant::now(), false));
                if !*said && since.elapsed() >= o.roster_grace {
                    *said = true;
                    if let Err(e) = write_error(&mut o.stdout, &not_on_roster(&o.session)) {
                        return Err(format!("attach: stdout closed: {e}"));
                    }
                }
            }
            End::Failed(Error::Status(se)) => (o.log)(&format!("attach: {se}")),
            End::Failed(e @ Error::Down(_)) => {
                (o.log)(&format!("attach: {e}"));
                if !outage {
                    outage = true;
                    if let Err(e) = write_error(&mut o.stdout, NOT_RUNNING) {
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

/// One `error` event line.
fn write_error(w: &mut Box<dyn Write + Send>, message: &str) -> std::io::Result<()> {
    let ev = Event {
        message: message.into(),
        ..Event::of(p::TYPE_ERROR)
    };
    // An Event holds only strings, ints and bools: marshal cannot fail.
    let line = gocompat::json::marshal(&ev).expect("attach event marshals");
    write_line(w, &line)
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
    if o.standby {
        pairs.push(("standby", "1"));
    }
    let opened = tokio::time::timeout(o.open_timeout, c.open_stream(&client::encode_query(&pairs)))
        .await
        .unwrap_or_else(|_| {
            Err(Error::Down(format!(
                "irori api /v1/attach: no answer in {:?}",
                o.open_timeout
            )))
        });
    let mut body = match opened {
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
        // Before hello the Core has OPEN_TIMEOUT to say it; after, a silent
        // stream (no keepalive either) is a lost one.
        let wait = if connected {
            o.idle_timeout
        } else {
            o.open_timeout
        };
        let Ok(frame) = tokio::time::timeout(wait, body.frame()).await else {
            (o.log)(&format!(
                "attach: nothing from irori in {wait:?}; reconnecting"
            ));
            if connected {
                return (connected, End::Ended);
            }
            return (
                connected,
                End::Failed(Error::Down(format!("irori sent no hello in {wait:?}"))),
            );
        };
        let frame = match frame {
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

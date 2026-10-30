//! `irori pair join` and `irori pair reset` (Go pair_cmd.go; surface.md
//! §1): this device joins another device's person. While Irori runs here
//! it drives that Core's join through the loopback API (`/v1/pair/*`);
//! otherwise it starts its own Core, for a machine without the desktop app
//! (the headless node).
//!
//! There is no Add side here: adding a device hands it the person secret
//! and every row, and is only in the desktop app's Settings → Devices.
//! Joining is safe to offer: it needs the person to type this device's
//! words into the other device's app, and the other device's key here. The
//! code and the key are read from prompts, never from argv, so they stay
//! out of shell history and process lists; stdin may be a pipe.

use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use irori_api::client::{Client, Error as ClientError};
use irori_core::{Core, Options, StartError};
use irori_pair::{MODE_JOIN, STATE_DONE, STATE_WAITING, STATE_WORDS, Status};
use tokio_util::sync::CancellationToken;

use crate::datadir;
use crate::host::{self, HostError, Hosted};

/// Go's `context.Canceled` text: the command was interrupted.
const CANCELED: &str = "context canceled";
const CLOSED: &str = "the pairing closed";

pub fn run(args: &[String]) -> Result<(), String> {
    if args.len() == 1 && args[0] == "reset" {
        return run_reset();
    }
    if args.first().is_none_or(|a| a != "join") {
        return Err("usage: irori pair join | irori pair reset (add a device from Settings → Devices in the desktop app)".into());
    }
    if args.len() > 1 {
        return Err(
            "usage: irori pair join (it asks for the code; don't pass it as an argument)".into(),
        );
    }
    let data = datadir::process_data_dir()?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let opts = host::options(&rt, data.clone(), true, None).map_err(|e| e.to_string())?;
    let lines = stdin_lines();
    let mut out = std::io::stdout();
    let (code, server) = ask_join(&lines, &mut out)?;
    // Only from here: at the prompts above Ctrl+C ends the command as
    // usual, and once a join is open it closes that join on the way out.
    let stop = CancellationToken::new();
    {
        let stop = stop.clone();
        rt.spawn(async move {
            interrupted().await;
            stop.cancel();
        });
    }
    let (side, own) = open_join(&rt, &data, opts)?;
    let res = join_loop(
        &side,
        &code,
        &server,
        &lines,
        &mut out,
        Duration::from_millis(300),
        &stop,
    );
    if let Some(h) = own {
        let _ = h.close();
    }
    res
}

/// Go `signal.NotifyContext(ctx, os.Interrupt, SIGTERM, SIGHUP)`.
async fn interrupted() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut term), Ok(mut hup)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) else {
            let _ = tokio::signal::ctrl_c().await;
            return;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
            _ = hup.recv() => {}
        }
    }
    #[cfg(windows)]
    {
        // Go: os.Interrupt is Ctrl+C or Ctrl+Break; SIGTERM is the console
        // closing.
        use tokio::signal::windows::{ctrl_break, ctrl_close};
        let (Ok(mut brk), Ok(mut close)) = (ctrl_break(), ctrl_close()) else {
            let _ = tokio::signal::ctrl_c().await;
            return;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = brk.recv() => {}
            _ = close.recv() => {}
        }
    }
}

fn say(out: &mut dyn Write, s: &str) {
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

/// Go `askJoin`: prompts for the code and, from away, the home server
/// address.
pub fn ask_join(lines: &Receiver<String>, out: &mut dyn Write) -> Result<(String, String), String> {
    say(
        out,
        "Pairing code (from Settings → Devices → Pair a device on the other device): ",
    );
    let code = match lines.recv() {
        Ok(c) if !c.trim_matches(gocompat::text::is_space).is_empty() => c,
        _ => return Err("no code typed".into()),
    };
    // At home the other device is found on the local network; from away,
    // the home server's address (shown under the code) gets this device in.
    say(
        out,
        "Home server address, only when joining from outside your home (Enter to skip): ",
    );
    let server = lines.recv().unwrap_or_default();
    let mut server = server.trim_matches(gocompat::text::is_space).to_string();
    if !server.is_empty() {
        server = home::hostport::host_port(&server, 0)
            .map_err(|_| irori_api::pair::BAD_SERVER.to_string())?;
    }
    Ok((code, server))
}

/// Why a typed key was not taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// Go `pair.ErrWrongKey`.
    Wrong,
    Other(String),
}

/// Go `joinSide`: the joining half of pairing, in the running Core through
/// its API or in a Core this command started.
pub trait JoinSide {
    fn join(&self, code: &str, server: &str) -> Result<Status, String>;
    fn status(&self) -> Result<Status, String>;
    /// Types the other device's key.
    fn key(&self, key: &str) -> Result<(), KeyError>;
    /// Closes the join, if it is still open.
    fn cancel(&self);
}

/// The side [`open_join`] picked.
pub enum Side {
    Api(ApiJoin),
    Local(LocalJoin),
}

impl JoinSide for Side {
    fn join(&self, code: &str, server: &str) -> Result<Status, String> {
        match self {
            Side::Api(s) => s.join(code, server),
            Side::Local(s) => s.join(code, server),
        }
    }
    fn status(&self) -> Result<Status, String> {
        match self {
            Side::Api(s) => s.status(),
            Side::Local(s) => s.status(),
        }
    }
    fn key(&self, key: &str) -> Result<(), KeyError> {
        match self {
            Side::Api(s) => s.key(key),
            Side::Local(s) => s.key(key),
        }
    }
    fn cancel(&self) {
        match self {
            Side::Api(s) => s.cancel(),
            Side::Local(s) => s.cancel(),
        }
    }
}

/// Go `openJoin`: the running Core when there is one, else a Core of its
/// own (with its API and lockfile, as Go's `core.Start` serves them), which
/// the caller closes.
pub fn open_join(
    rt: &tokio::runtime::Runtime,
    data: &Path,
    opts: Options,
) -> Result<(Side, Option<Hosted>), String> {
    match rt.block_on(Client::dial(data)) {
        Ok(c) => {
            return Ok((
                Side::Api(ApiJoin {
                    c,
                    rt: rt.handle().clone(),
                }),
                None,
            ));
        }
        // An Irori that answered with an error is running; anything else
        // (no lockfile, a dead pid, an unreadable irori.json) goes on to
        // start a Core, which refuses itself if one does run.
        Err(e @ ClientError::Status { .. }) => return Err(e.to_string()),
        Err(_) => {}
    }
    let h = pair_core(rt, opts)?;
    Ok((Side::Local(LocalJoin(h.core.clone())), Some(h)))
}

/// Go `pairCore`: a headless Core for `irori pair join` when Irori isn't
/// running here.
fn pair_core(rt: &tokio::runtime::Runtime, opts: Options) -> Result<Hosted, String> {
    match host::start_with(rt, opts) {
        Ok(h) => Ok(h),
        // It started between the API check and here, or it runs but its
        // API doesn't answer.
        Err(HostError::Start(StartError::AlreadyRunning { .. })) => Err(
            "Irori is running on this device but didn't answer: run `irori pair join` again, or quit Irori first"
                .into(),
        ),
        Err(e) => Err(e.to_string()),
    }
}

fn same_instant(a: &Option<gocompat::time::GoTime>, b: &Option<gocompat::time::GoTime>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.secs == b.secs && a.nanos == b.nanos,
        (None, None) => true,
        _ => false,
    }
}

enum Wait {
    Line(String),
    Closed,
    Canceled,
    Timeout,
}

/// Go's `select` on the lines, the context and `time.After(every)`;
/// `lines: None` is Go's nil channel (never ready).
fn wait(lines: Option<&Receiver<String>>, stop: &CancellationToken, every: Duration) -> Wait {
    let end = Instant::now() + every;
    loop {
        if stop.is_cancelled() {
            return Wait::Canceled;
        }
        let now = Instant::now();
        if now >= end {
            return Wait::Timeout;
        }
        let step = (end - now).min(Duration::from_millis(25));
        match lines {
            Some(l) => match l.recv_timeout(step) {
                Ok(s) => return Wait::Line(s),
                Err(RecvTimeoutError::Disconnected) => return Wait::Closed,
                Err(RecvTimeoutError::Timeout) => {}
            },
            None => std::thread::sleep(step),
        }
    }
}

/// Go `joinLoop`: joins with `code`, prints the words for the person to
/// type on the other device, takes the other device's key from `lines` and
/// follows the pairing until it is done, failed or expired. Once its join
/// is open, every way out but "Paired." closes it, so the running Core
/// isn't left holding a join nobody follows (and refusing the app's own
/// Add or Join as busy).
pub fn join_loop(
    side: &dyn JoinSide,
    code: &str,
    server: &str,
    lines: &Receiver<String>,
    out: &mut dyn Write,
    every: Duration,
    stop: &CancellationToken,
) -> Result<(), String> {
    let first = side.join(code, server)?;
    let mut ours = true;
    let res = follow(side, &first, lines, out, every, stop, &mut ours);
    // Not when the status stopped being this join: that pairing is someone
    // else's (the app's), and it isn't this command's to close.
    if res.is_err() && ours {
        side.cancel();
    }
    res
}

fn follow(
    side: &dyn JoinSide,
    first: &Status,
    lines: &Receiver<String>,
    out: &mut dyn Write,
    every: Duration,
    stop: &CancellationToken,
    ours: &mut bool,
) -> Result<(), String> {
    say(out, "Looking for the other device...\n");
    let mut shown = false;
    loop {
        let st = match side.status() {
            Ok(st) => st,
            Err(e) => {
                if stop.is_cancelled() {
                    return Err(CANCELED.into());
                }
                return Err(e);
            }
        };
        // The running Core has one pairing at a time, shared with the app:
        // once this join ended, the status may be a newer Add or Join.
        if st.mode != MODE_JOIN || !same_instant(&st.expires, &first.expires) {
            *ours = false;
            return Err(CLOSED.into());
        }
        match st.state.as_str() {
            STATE_WORDS => {
                let who = if st.peer.is_empty() {
                    "the other device".to_string()
                } else {
                    st.peer.clone()
                };
                if !shown {
                    say(
                        out,
                        &format!(
                            "Type these words on {who}:\n\n    {}\n\n",
                            st.words.join(" ")
                        ),
                    );
                    say(
                        out,
                        &format!("Then type the key {who} shows beside its code: "),
                    );
                    shown = true;
                }
                if !st.key_ok {
                    match wait(Some(lines), stop, every) {
                        Wait::Line(line) => match side.key(&line) {
                            Err(KeyError::Wrong) => {
                                if side.status().is_ok_and(|s| s.state == STATE_WORDS) {
                                    say(out, "That isn't the key. Try once more: ");
                                }
                            }
                            Err(KeyError::Other(e)) => return Err(e),
                            Ok(()) => say(out, &format!("Waiting for {who} (Ctrl+C cancels).\n")),
                        },
                        Wait::Closed => return Err("no key typed".into()),
                        Wait::Canceled => return Err(CANCELED.into()),
                        Wait::Timeout => {}
                    }
                    continue;
                }
            }
            STATE_DONE => {
                say(
                    out,
                    "Paired. Keep Irori running here (or `irori serve --headless`) to keep the devices in sync.\n",
                );
                return Ok(());
            }
            STATE_WAITING => {}
            _ => {
                *ours = false; // already closed: nothing to cancel
                if st.error.is_empty() {
                    return Err(CLOSED.into());
                }
                return Err(st.error);
            }
        }
        if let Wait::Canceled = wait(None, stop, every) {
            return Err(CANCELED.into());
        }
    }
}

/// Go `localJoin`: the join in a Core this command started.
pub struct LocalJoin(pub Arc<Core>);

impl JoinSide for LocalJoin {
    fn join(&self, code: &str, server: &str) -> Result<Status, String> {
        let c = &self.0;
        c.pairing_runtime()
            .block_on(c.pairing().join_via(code, server))
            .map_err(|e| e.to_string())
    }
    fn status(&self) -> Result<Status, String> {
        Ok(self.0.pairing().status())
    }
    fn key(&self, key: &str) -> Result<(), KeyError> {
        let c = &self.0;
        match c.pairing_runtime().block_on(c.pairing().confirm_key(key)) {
            Ok(()) => Ok(()),
            Err(irori_pair::Error::WrongKey) => Err(KeyError::Wrong),
            Err(e) => Err(KeyError::Other(e.to_string())),
        }
    }
    fn cancel(&self) {
        self.0.pairing().cancel_join();
    }
}

/// Go `apiJoin`: the join in the running Core, over its loopback API.
pub struct ApiJoin {
    c: Client,
    rt: tokio::runtime::Handle,
}

/// Go `apiText`: the server's own message for a refused call (it never
/// carries the code or the key), without the route and status around it.
fn api_text(e: ClientError) -> String {
    match e {
        ClientError::Status { message, .. } if !message.is_empty() => message,
        e => e.to_string(),
    }
}

impl JoinSide for ApiJoin {
    fn join(&self, code: &str, server: &str) -> Result<Status, String> {
        self.rt
            .block_on(self.c.pair_join(code, server))
            .map_err(|e| {
                if irori_api::pair::is_no_route(&e) {
                    "the Irori running on this device is older than this command: update it, or quit it and run `irori pair join` again".into()
                } else {
                    api_text(e)
                }
            })
    }
    fn status(&self) -> Result<Status, String> {
        self.rt.block_on(self.c.pair_status()).map_err(api_text)
    }
    fn key(&self, key: &str) -> Result<(), KeyError> {
        match self.rt.block_on(self.c.pair_key(key)) {
            Ok(_) => Ok(()),
            Err(e) if irori_api::pair::is_wrong_key(&e) => Err(KeyError::Wrong),
            Err(e) => Err(KeyError::Other(api_text(e))),
        }
    }
    /// Runs even after an interrupt, so it has its own deadline.
    fn cancel(&self) {
        let _ = self.rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), self.c.pair_cancel()).await
        });
    }
}

/// Go `runPairReset`: this device's log, device id and keys move aside,
/// and it starts next time as a new device that can pair again. Only the
/// settings stay; nothing is deleted.
fn run_reset() -> Result<(), String> {
    let data = datadir::process_data_dir()?;
    host::claude_dir()?; // Go's coreOptions fails without a home dir too
    let mut out = std::io::stdout();
    say(
        &mut out,
        "This makes this device a new one: its event log, device id and keys move aside, and only the settings stay.\nType reset to go on: ",
    );
    match stdin_lines().recv() {
        Ok(l) if l.trim_matches(gocompat::text::is_space) == "reset" => {}
        _ => return Err("not reset".into()),
    }
    let dst = irori_core::reset::reset(&data)?;
    say(
        &mut out,
        &format!(
            "Done. The old files are in {}. Start Irori to pair this device again.\n",
            dst.display()
        ),
    );
    Ok(())
}

/// Go `stdinLines`: the lines the person types.
fn stdin_lines() -> Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new()
        .name("irori-stdin".into())
        .spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    rx
}

#[cfg(test)]
mod tests;

//! Hosting one Core with its loopback API, shared by `irori serve
//! --headless` and the window (Go `core.Start`, whose Core serves its own
//! API; here the api crate serves it, architecture.md 1.1).

use std::path::PathBuf;
use std::sync::Arc;

use irori_core::{Core, Options, StartError};
use tokio_util::sync::CancellationToken;

use crate::commands::version;

/// Go `paths.ClaudeDir`: `CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn claude_dir() -> Result<PathBuf, String> {
    if let Some(d) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(d));
    }
    let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(home)
        .filter(|v| !v.is_empty())
        .map(|h| PathBuf::from(h).join(".claude"))
        .ok_or_else(|| "paths: no home directory".to_string())
}

/// Whether `IRORI_DEV_OFFLINE=1` asks for a Core with no network
/// (`irori_core::offline`).
pub fn offline() -> bool {
    irori_core::offline::requested(std::env::var(irori_core::offline::ENV).ok().as_deref())
}

/// A running Core and its API server.
pub struct Hosted {
    pub core: Arc<Core>,
    /// The loopback API's port (the lockfile's).
    pub port: u16,
    stop: CancellationToken,
    serving: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
    rt: tokio::runtime::Handle,
}

/// Why [`start`] failed: the Core's own error (the window tells
/// `AlreadyRunning` apart), or the API's.
pub enum HostError {
    Start(StartError),
    Api(String),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::Start(e) => e.fmt(f),
            HostError::Api(s) => f.write_str(s),
        }
    }
}

/// Starts the Core for `data` on `rt`, serves its API on 127.0.0.1 and
/// writes the lockfile once the listener is open and the services run, as
/// Go does. `notifier` is the desktop app's cards (`None`: headless).
pub fn start(
    rt: &tokio::runtime::Runtime,
    data: PathBuf,
    headless: bool,
    notifier: Option<notify::Notifier>,
) -> Result<Hosted, HostError> {
    start_with(rt, options(rt, data, headless, notifier)?)
}

/// Go `coreOptions`: the Core's options for `data` on `rt`.
pub fn options(
    rt: &tokio::runtime::Runtime,
    data: PathBuf,
    headless: bool,
    notifier: Option<notify::Notifier>,
) -> Result<Options, HostError> {
    let claude = claude_dir().map_err(HostError::Api)?;
    let mut opts = Options {
        data_dir: data,
        claude_dir: claude,
        version: version().into(),
        headless,
        runtime: Some(rt.handle().clone()),
        notifier,
        ..Options::default()
    };
    if offline() {
        // A rehearsal on a copy (tools/rehearse): the copy holds this
        // device's real key, so its Core must never reach the network.
        // Honoured in every build, since it can only take the network away.
        opts.pair_net = Some(irori_core::offline::offline_net());
        opts.no_home = true;
    }
    Ok(opts)
}

/// [`start`] with the Core's options given (tests: fixture identities and
/// loopback networks).
pub fn start_with(rt: &tokio::runtime::Runtime, opts: Options) -> Result<Hosted, HostError> {
    let core = Arc::new(Core::start(opts).map_err(HostError::Start)?);
    let stop = CancellationToken::new();
    let served = rt.block_on(async {
        let server = irori_api::Server::new(core.clone(), core.token())
            .with_tools(Arc::new(irori_api::CoreTools(core.clone())));
        let ln = irori_api::listen().await.map_err(|e| e.to_string())?;
        let port = ln.local_addr().map_err(|e| e.to_string())?.port();
        let serving = {
            let stop = stop.clone();
            tokio::spawn(async move { server.serve(ln, stop).await })
        };
        core.write_lockfile(port).map_err(|e| e.to_string())?;
        Ok::<_, String>((serving, port))
    });
    match served {
        Ok((serving, port)) => Ok(Hosted {
            core,
            port,
            stop,
            serving: Some(serving),
            rt: rt.handle().clone(),
        }),
        Err(e) => {
            stop.cancel();
            let _ = core.close();
            Err(HostError::Api(e))
        }
    }
}

impl Hosted {
    /// Stops the API, then closes the Core (Go's 5 s close wait).
    pub fn close(mut self) -> Result<(), String> {
        self.stop.cancel();
        if let Some(s) = self.serving.take() {
            let _ = self.rt.block_on(s);
        }
        self.core.close().map_err(|e| e.to_string())
    }
}

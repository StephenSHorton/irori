//! `irori-update`: the in-app updater, a port of Go `internal/update` at
//! be67a86 (surface.md §3 "Updater rules", architecture.md §7.3, D11).
//!
//! It reads the latest GitHub release of [`REPO`], compares its tag with the
//! running version, downloads this platform's asset, checks it against the
//! release's `SHA256SUMS`, and parks in `ready`. The person then restarts
//! ([`Updater::restart_to_update`]: install, quit, Irori comes back) or keeps
//! working, and the install runs on quit ([`Updater::install_on_exit`], no
//! relaunch). Nothing installs or quits behind the person's back, and `dev`
//! builds never touch the network.
//!
//! The feed and the asset names are Go's, so the Go v0.5.x updater installs
//! the first Rust release (v0.6.0) and this one installs later ones:
//! `irori-<v>-windows-amd64-installer.exe` (the NSIS installer built from
//! app/nsis/irori.nsi by scripts/release.ps1), `irori-<v>-macos-arm64.app.zip`
//! (app/macos/package.sh) and `SHA256SUMS` (`sha256sum` format).
//! `tauri-plugin-updater` is not used: it wants its own signed manifest,
//! which the Go updater cannot read.
//!
//! | Module | Go |
//! |---|---|
//! | [`version`] | `isNewer`, `cmpSemver`, `parseVer`, `numericVersion` |
//! | [`release`] | release.go: asset matchers, `SHA256SUMS` lines |
//! | [`nsis`] | nsis.go: `/S [/NORELAUNCH] /WAITPID=<pid>`, `CleanupOldExes` |
//! | [`macapp`] | macapp.go: bundle paths, the swap helper script |
//! | `mac` (macOS only) | mac_darwin.go: unpack, bundle and signature checks, staging |
//! | [`login`] | the launch-at-login cutover check (architecture.md D9, surface.md §7.4) |
//!
//! Wiring (for the app and the window crate's `Updater` seam):
//! - Build one [`Updater`] in the GUI role only: `Updater::new(REPO,
//!   version, <data dir>/updates)`; `serve --headless` and `mcp` never
//!   update (Go releasing.md).
//! - `set_hooks(emit, quit)`: `emit` sends the `update:status` window event
//!   ([`EVENT_UPDATE_STATUS`], payload [`Status`]); `quit` quits the app.
//!   `set_on_change` rebuilds the tray (`Tray::set_update`) and posts the
//!   update card (`update:<v>`, "Irori v<v> is ready", Install / Later).
//! - At GUI start, spawn [`Updater::auto_update`]; in the exit path call
//!   [`Updater::install_on_exit`].
//! - `with_logger(irori_core::log)`: the updater's lines (check, download,
//!   verify and launch failures) go to `irori.log` like Go's `log.Printf`;
//!   unset, they go to stderr, which a GUI-subsystem exe has none of.
//! - The async methods need a Tokio runtime; the window seam's sync
//!   `check_and_download` waits for them off the async workers.

pub mod login;
#[cfg(target_os = "macos")]
pub mod mac;
pub mod macapp;
pub mod nsis;
pub mod release;
pub mod version;

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};

use release::{GhAsset, GhRelease};

/// Go `Repo`: the repository Irori updates from.
pub const REPO: &str = "StephenSHorton/irori";

/// Go `EventUpdateStatus`.
pub const EVENT_UPDATE_STATUS: &str = "update:status";

/// Go's phases, in the order of a normal run.
pub const PHASE_IDLE: &str = "idle";
pub const PHASE_CHECKING: &str = "checking";
pub const PHASE_DOWNLOADING: &str = "downloading";
pub const PHASE_READY: &str = "ready";
pub const PHASE_INSTALLING: &str = "installing";
pub const PHASE_ERROR: &str = "error";

/// Go `supported`: Windows and macOS update themselves.
pub const SUPPORTED: bool = cfg!(any(windows, target_os = "macos"));

/// Go `Status`, as the window and tray see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub phase: String,
    pub version: String,
    pub notes: String,
    /// The verified installer (or staged bundle) once ready; not sent to
    /// the window.
    #[serde(skip)]
    pub installer_path: PathBuf,
    pub error: String,
}

impl Status {
    fn phase(p: &str) -> Status {
        Status {
            phase: p.into(),
            version: String::new(),
            notes: String::new(),
            installer_path: PathBuf::new(),
            error: String::new(),
        }
    }
}

/// Go `UpdateInfo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    pub asset_url: String,
    pub asset_name: String,
    pub sha256: String,
    pub published_at: String,
}

/// Starts a program detached (the installer or the macOS swap helper).
pub type Launcher = Arc<dyn Fn(&Path, &[String]) -> std::io::Result<()> + Send + Sync>;
type Emit = Arc<dyn Fn(&str, &Status) + Send + Sync>;
type Quit = Arc<dyn Fn() + Send + Sync>;
type OnChange = Arc<dyn Fn(&Status) + Send + Sync>;
/// Where the updater's diagnostics go (Go `log.Printf`).
pub type Logger = Arc<dyn Fn(&str) + Send + Sync>;
/// Finds the running Irori.app (Go `macPlatform`'s `app`): tests point it
/// at a fake bundle.
pub type RunningApp = Arc<dyn Fn() -> Result<PathBuf, String> + Send + Sync>;

/// What updating means on one OS (Go `platform`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    /// The NSIS installer (Windows; usable anywhere in tests).
    Nsis,
    /// The `.app` swap (macOS only).
    Mac,
    /// No self-update (Linux and the rest).
    Unsupported,
}

impl Platform {
    /// This OS's platform (Go `newPlatform`).
    pub fn native() -> Platform {
        if cfg!(windows) {
            Platform::Nsis
        } else if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Unsupported
        }
    }

    fn asset_kind(self) -> String {
        match self {
            Platform::Nsis => "windows installer".into(),
            Platform::Mac => format!("macOS {} app", release::goarch()),
            Platform::Unsupported => std::env::consts::OS.into(),
        }
    }

    fn is_asset(self, name: &str) -> bool {
        match self {
            Platform::Nsis => release::is_installer_asset(name),
            Platform::Mac => release::is_mac_app_asset(name, release::goarch()),
            Platform::Unsupported => false,
        }
    }
}

struct Inner {
    status: Status,
    emit: Option<Emit>,
    quit: Option<Quit>,
    on_change: Option<OnChange>,
}

/// Go `UpdateService`.
pub struct Updater {
    repo: String,
    current: String,
    api_base: String,
    dir: PathBuf,
    plat: Platform,
    supported: bool,
    launch: Launcher,
    pid: u32,
    log: Logger,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    running_app: Option<RunningApp>,
    inner: Mutex<Inner>,
    in_flight: AtomicBool,
}

impl Updater {
    /// An updater for `repo` (normally [`REPO`]) at the running version,
    /// downloading into `dir` (Go: `<data dir>/updates`).
    pub fn new(repo: &str, current: &str, dir: impl Into<PathBuf>) -> Updater {
        Updater {
            repo: repo.into(),
            current: current.into(),
            api_base: "https://api.github.com".into(),
            dir: dir.into(),
            plat: Platform::native(),
            supported: SUPPORTED,
            launch: Arc::new(launch_detached),
            pid: std::process::id(),
            log: Arc::new(|l: &str| eprintln!("{l}")),
            running_app: None,
            inner: Mutex::new(Inner {
                status: Status::phase(PHASE_IDLE),
                emit: None,
                quit: None,
                on_change: None,
            }),
            in_flight: AtomicBool::new(false),
        }
    }

    /// Replaces the GitHub API root (tests: a loopback fake).
    pub fn with_api_base(mut self, base: &str) -> Updater {
        self.api_base = base.trim_end_matches('/').into();
        self
    }

    /// Replaces the launcher (tests: a recorder).
    pub fn with_launcher(mut self, launch: Launcher) -> Updater {
        self.launch = launch;
        self
    }

    /// Sends the diagnostics to `log` (the app: `irori_core::log`, so they
    /// reach `irori.log`).
    pub fn with_logger(mut self, log: Logger) -> Updater {
        self.log = log;
        self
    }

    /// Replaces the pid the installer or swap helper waits for (tests: a
    /// stand-in GUI process).
    pub fn with_pid(mut self, pid: u32) -> Updater {
        self.pid = pid;
        self
    }

    /// Replaces how the running Irori.app is found (macOS; tests: a fake
    /// bundle, as Go's `macPlatform(s, app, pid)`).
    pub fn with_running_app(mut self, f: RunningApp) -> Updater {
        self.running_app = Some(f);
        self
    }

    fn logf(&self, line: &str) {
        (self.log)(line);
    }

    #[cfg(target_os = "macos")]
    fn live_app(&self) -> Result<PathBuf, String> {
        match &self.running_app {
            Some(f) => f(),
            None => mac::running_app(),
        }
    }

    /// Replaces the platform; [`Platform::Nsis`] marks the updater supported
    /// so its paths run on any OS, as Go's tests swap in `nsisPlatform`.
    pub fn with_platform(mut self, plat: Platform) -> Updater {
        self.plat = plat;
        self.supported = match plat {
            Platform::Nsis => true,
            Platform::Mac => cfg!(target_os = "macos"),
            Platform::Unsupported => false,
        };
        self
    }

    /// Sets the status directly (tests: Go's `readyService`).
    #[doc(hidden)]
    pub fn force_status(&self, st: Status) {
        self.inner.lock().unwrap().status = st;
    }

    /// Go `SetHooks`.
    pub fn set_hooks(&self, emit: Emit, quit: Quit) {
        let mut g = self.inner.lock().unwrap();
        g.emit = Some(emit);
        g.quit = Some(quit);
    }

    /// Go `SetOnChange`: fired after every status transition.
    pub fn set_on_change(&self, f: OnChange) {
        self.inner.lock().unwrap().on_change = Some(f);
    }

    /// Go `GetVersion`.
    pub fn version(&self) -> &str {
        &self.current
    }

    /// Go `Supported`.
    pub fn supported(&self) -> bool {
        self.supported
    }

    /// Go `GetStatus`.
    pub fn status(&self) -> Status {
        self.inner.lock().unwrap().status.clone()
    }

    /// Mutates the status under the lock, then tells listeners outside it.
    fn set_status(&self, f: impl FnOnce(&mut Status)) -> Status {
        let (st, emit, cb) = {
            let mut g = self.inner.lock().unwrap();
            f(&mut g.status);
            (g.status.clone(), g.emit.clone(), g.on_change.clone())
        };
        if let Some(e) = emit {
            e(EVENT_UPDATE_STATUS, &st);
        }
        if let Some(cb) = cb {
            cb(&st);
        }
        st
    }

    fn unsupported_error(&self) -> String {
        format!("updates not supported on {} yet", std::env::consts::OS)
    }

    /// Go `CheckForUpdate`: the update on offer, or `None` when current (or
    /// a dev build).
    pub async fn check_for_update(&self) -> Result<Option<UpdateInfo>, String> {
        if !self.supported {
            return Err(self.unsupported_error());
        }
        if version::is_dev(&self.current) {
            return Ok(None);
        }
        #[cfg(target_os = "macos")]
        if self.plat == Platform::Mac {
            self.live_app()?;
        }
        self.find_update().await
    }

    /// Go `AutoUpdate`: the startup check. Cleans up after an earlier
    /// update, then checks and downloads; errors are only logged.
    pub async fn auto_update(&self) {
        if !self.supported {
            self.logf(&format!("irori: {}", self.unsupported_error()));
            return;
        }
        self.cleanup();
        if let Err(e) = self.check_and_download().await {
            self.logf(&format!("irori: auto-update failed: {e}"));
        }
    }

    fn cleanup(&self) {
        match self.plat {
            Platform::Nsis => {
                if let Ok(exe) = std::env::current_exe() {
                    nsis::cleanup_old_exes(&exe);
                }
            }
            #[cfg(target_os = "macos")]
            Platform::Mac => {
                if let Ok(live) = self.live_app() {
                    mac::cleanup(&live);
                }
            }
            _ => {}
        }
    }

    /// Go `CheckAndDownload`: check, download, verify, park in ready. One
    /// run at a time; a concurrent caller gets the current status. Ready or
    /// installing is returned as is.
    pub async fn check_and_download(&self) -> Result<Status, String> {
        if self
            .in_flight
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(self.status());
        }
        struct Clear<'a>(&'a AtomicBool);
        impl Drop for Clear<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _clear = Clear(&self.in_flight);

        let st = self.status();
        if st.phase == PHASE_READY || st.phase == PHASE_INSTALLING {
            return Ok(st);
        }
        self.set_status(|s| *s = Status::phase(PHASE_CHECKING));
        let info = match self.check_for_update().await {
            Ok(Some(i)) => i,
            Ok(None) => return Ok(self.set_status(|s| *s = Status::phase(PHASE_IDLE))),
            Err(e) => {
                self.fail(&e);
                return Err(e);
            }
        };
        self.logf(&format!("irori: downloading update v{}", info.version));
        self.set_status(|s| {
            *s = Status {
                version: info.version.clone(),
                notes: info.notes.clone(),
                ..Status::phase(PHASE_DOWNLOADING)
            }
        });
        let path = match self.download_verified(&info).await {
            Ok(p) => p,
            Err(e) => {
                self.fail(&e);
                return Err(e);
            }
        };
        let path = match self.stage(&path, &info) {
            Ok(p) => p,
            Err(e) => {
                self.fail(&e);
                return Err(e);
            }
        };
        let st = self.set_status(|s| {
            s.phase = PHASE_READY.into();
            s.installer_path = path.clone();
            s.error.clear();
        });
        self.logf(&format!(
            "irori: update v{} ready at {}",
            info.version,
            path.display()
        ));
        Ok(st)
    }

    #[allow(unused_variables)]
    fn stage(&self, downloaded: &Path, info: &UpdateInfo) -> Result<PathBuf, String> {
        #[cfg(target_os = "macos")]
        if self.plat == Platform::Mac {
            return mac::stage(&self.live_app()?, &self.dir, downloaded, &info.version);
        }
        Ok(downloaded.to_path_buf())
    }

    fn fail(&self, err: &str) -> Status {
        self.logf(&format!("irori: update: {err}"));
        self.set_status(|s| {
            s.phase = PHASE_ERROR.into();
            s.error = err.into();
            s.installer_path = PathBuf::new();
        })
    }

    /// Starts installing `staged`; the caller then quits.
    fn install(&self, staged: &Path, relaunch: bool) -> Result<(), String> {
        match self.plat {
            Platform::Nsis => {
                (self.launch)(staged, &nsis::install_args(relaunch)).map_err(|e| e.to_string())
            }
            #[cfg(target_os = "macos")]
            Platform::Mac => {
                let live = self.live_app()?;
                let (sh, args) =
                    mac::install_command(&live, &self.dir, staged, relaunch, self.pid)?;
                (self.launch)(&sh, &args).map_err(|e| e.to_string())
            }
            _ => Err(self.unsupported_error()),
        }
    }

    /// Go `RestartToUpdate`: start the install and quit. Requires ready;
    /// flips to installing before quitting so [`Updater::install_on_exit`]
    /// sees it; a launch failure goes back to ready.
    pub fn restart_to_update(&self) -> Result<(), String> {
        let (path, quit) = {
            let mut g = self.inner.lock().unwrap();
            if g.status.phase != PHASE_READY {
                return Err(format!(
                    "no update ready to install (phase {})",
                    go_quote(&g.status.phase)
                ));
            }
            g.status.phase = PHASE_INSTALLING.into();
            (g.status.installer_path.clone(), g.quit.clone())
        };
        if let Err(e) = self.install(&path, true) {
            self.set_status(|s| s.phase = PHASE_READY.into());
            return Err(format!("launch installer: {e}"));
        }
        self.logf(&format!(
            "irori: restarting to update (installer {})",
            path.display()
        ));
        self.set_status(|_| {});
        if let Some(q) = quit {
            q();
        }
        Ok(())
    }

    /// Go `InstallOnExit`: from ready only, install without relaunch.
    pub fn install_on_exit(&self) {
        let path = {
            let mut g = self.inner.lock().unwrap();
            if g.status.phase != PHASE_READY {
                return;
            }
            g.status.phase = PHASE_INSTALLING.into();
            g.status.installer_path.clone()
        };
        if let Err(e) = self.install(&path, false) {
            self.logf(&format!("irori: install on exit: launch installer: {e}"));
            return;
        }
        self.logf(&format!(
            "irori: installing update on exit (installer {})",
            path.display()
        ));
    }

    /// The pid the installer or swap helper waits for (this process).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    fn client(&self, timeout: Option<Duration>) -> Result<reqwest::Client, String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        use rustls_platform_verifier::BuilderVerifierExt as _;
        let tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_platform_verifier()
            .map_err(|e| e.to_string())?
            .with_no_client_auth();
        let mut b = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .user_agent(format!("irori/{}", self.current));
        if let Some(t) = timeout {
            b = b.timeout(t);
        }
        b.build().map_err(|e| e.to_string())
    }

    /// Go `latestRelease`: `None` when the repo has no releases (404).
    async fn latest_release(&self, client: &reqwest::Client) -> Result<Option<GhRelease>, String> {
        let url = format!("{}/repos/{}/releases/latest", self.api_base, self.repo);
        let resp = client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body = resp.bytes().await.map_err(|e| e.to_string())?;
        if status != reqwest::StatusCode::OK {
            let b = &body[..body.len().min(512)];
            return Err(format!(
                "github releases: {}: {}",
                status,
                String::from_utf8_lossy(b).trim()
            ));
        }
        serde_json::from_slice(&body)
            .map(Some)
            .map_err(|e| e.to_string())
    }

    /// Go `findUpdate`, under Go's 30 s bound for the API and checksum calls.
    async fn find_update(&self) -> Result<Option<UpdateInfo>, String> {
        if version::is_dev(&self.current) {
            return Ok(None);
        }
        match tokio::time::timeout(Duration::from_secs(30), self.find_update_inner()).await {
            Ok(r) => r,
            Err(_) => Err("context deadline exceeded".into()),
        }
    }

    async fn find_update_inner(&self) -> Result<Option<UpdateInfo>, String> {
        let client = self.client(Some(Duration::from_secs(30)))?;
        let Some(rel) = self.latest_release(&client).await? else {
            return Ok(None);
        };
        if !version::is_newer(&rel.tag_name, &self.current) {
            return Ok(None);
        }
        let mut installer = GhAsset::default();
        let mut sums_url = String::new();
        for a in &rel.assets {
            if self.plat.is_asset(&a.name) {
                installer = a.clone();
            }
            if release::is_checksum_asset(&a.name) {
                sums_url = a.url.clone();
            }
        }
        if installer.url.is_empty() {
            return Err(format!(
                "release {} has no {} asset",
                rel.tag_name,
                self.plat.asset_kind()
            ));
        }
        if sums_url.is_empty() {
            return Err(format!("release {} has no SHA256SUMS asset", rel.tag_name));
        }
        let sum = self
            .fetch_checksum(&client, &sums_url, &installer.name)
            .await?;
        Ok(Some(UpdateInfo {
            version: rel
                .tag_name
                .strip_prefix('v')
                .unwrap_or(&rel.tag_name)
                .to_string(),
            notes: rel.body,
            asset_url: installer.url,
            asset_name: installer.name,
            sha256: sum,
            published_at: rel.published_at,
        }))
    }

    /// Go `fetchChecksum`: reads at most 1 MiB of `SHA256SUMS`.
    async fn fetch_checksum(
        &self,
        client: &reqwest::Client,
        url: &str,
        name: &str,
    ) -> Result<String, String> {
        let mut resp = client.get(url).send().await.map_err(|e| e.to_string())?;
        if resp.status() != reqwest::StatusCode::OK {
            return Err(format!("download SHA256SUMS: {}", resp.status()));
        }
        let mut b = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            b.extend_from_slice(&chunk);
            if b.len() >= 1 << 20 {
                b.truncate(1 << 20);
                break;
            }
        }
        release::checksum_for(&String::from_utf8_lossy(&b), name)
            .ok_or_else(|| format!("SHA256SUMS has no line for {name}"))
    }

    /// Go `downloadVerified`: into `dir/<base name>`, dropping earlier
    /// downloads (but not `*.log` or folders); a hash mismatch deletes it.
    pub async fn download_verified(&self, info: &UpdateInfo) -> Result<PathBuf, String> {
        if info.asset_name.is_empty() || info.asset_url.is_empty() {
            return Err("update has no installer asset".into());
        }
        if info.sha256.is_empty() {
            return Err("update has no checksum".into());
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let dst = self.dir.join(release::base(&info.asset_name));
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let p = self.dir.join(e.file_name());
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                // Go `filepath.Ext(name) == ".log"`, which a bare `.log` passes.
                let is_log = e.file_name().to_string_lossy().ends_with(".log");
                if p != dst && !is_dir && !is_log {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
        let dl = tokio::time::timeout(
            Duration::from_secs(600),
            self.download(&info.asset_url, &dst),
        );
        match dl.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = std::fs::remove_file(&dst);
                return Err(e);
            }
            Err(_) => {
                let _ = std::fs::remove_file(&dst);
                return Err("context deadline exceeded".into());
            }
        }
        let got = sha256_file(&dst).map_err(|e| e.to_string())?;
        if !got.eq_ignore_ascii_case(&info.sha256) {
            let _ = std::fs::remove_file(&dst);
            return Err(format!(
                "update checksum mismatch: got {got}, want {}",
                info.sha256
            ));
        }
        Ok(dst)
    }

    async fn download(&self, url: &str, dst: &Path) -> Result<(), String> {
        use std::io::Write as _;
        let client = self.client(None)?;
        let mut resp = client.get(url).send().await.map_err(|e| e.to_string())?;
        if resp.status() != reqwest::StatusCode::OK {
            return Err(format!("download {url}: {}", resp.status()));
        }
        let mut f = std::fs::File::create(dst).map_err(|e| e.to_string())?;
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            f.write_all(&chunk).map_err(|e| e.to_string())?;
        }
        f.sync_all().map_err(|e| e.to_string())
    }
}

fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Go `%q` for the phase names (plain ASCII words).
fn go_quote(s: &str) -> String {
    format!("{s:?}")
}

/// Go's launcher: start `path args...` and do not wait. On Windows a child
/// of a GUI process already outlives it; on Unix the swap helper gets its
/// own session (Go `detach`, `Setsid`), so closing the terminal that
/// started `irori` does not take it along.
pub fn launch_detached(path: &Path, args: &[String]) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(path);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // SAFETY: setsid is async-signal-safe.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    cmd.spawn().map(|_| ())
}

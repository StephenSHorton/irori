use std::path::Path;
use std::time::SystemTime;

fn main() {
    stale_frontend_guard();
    // Replace the manifest tauri-build embeds with the Go build's equivalent
    // (architecture.md Critic finding 4): per-monitor-v2 DPI matters to the
    // native toast windows that come later.
    let windows =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("irori.exe.manifest"));
    let attrs = tauri_build::Attributes::new().windows_attributes(windows);
    if let Err(e) = tauri_build::try_build(attrs) {
        panic!("tauri-build: {e:#}");
    }
}

/// generate_context! embeds frontend/dist as it is. A dist older than the
/// frontend sources is a build of other code: on 2026-10-30 a dev exe
/// embedded a pre-M6 dist whose host check was a constant false, so the
/// window ran on the sample data and showed a sample "New laptop wants to
/// join". Refuse that build instead of shipping the old window.
fn stale_frontend_guard() {
    let fe = Path::new(env!("CARGO_MANIFEST_DIR")).join("../frontend");
    let built = fe.join("dist/index.html");
    let sources = [fe.join("src"), fe.join("public"), fe.join("index.html")];
    println!("cargo:rerun-if-changed={}", built.display());
    for s in &sources {
        println!("cargo:rerun-if-changed={}", s.display());
    }
    let Some(built_at) = mtime(&built) else {
        panic!(
            "frontend/dist is missing: run `bun run build` in frontend/ first \
             (scripts/check.sh does)"
        );
    };
    if let Some((newest, path)) = sources.iter().filter_map(|s| newest(s)).max()
        && newest > built_at
    {
        panic!(
            "frontend/dist is older than {}: run `bun run build` in \
             frontend/ so the exe doesn't embed a stale window",
            path.display()
        );
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// The newest modification time under `p` (a file or a directory tree).
fn newest(p: &Path) -> Option<(SystemTime, std::path::PathBuf)> {
    let md = std::fs::metadata(p).ok()?;
    if !md.is_dir() {
        return Some((md.modified().ok()?, p.to_path_buf()));
    }
    std::fs::read_dir(p)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| newest(&e.path()))
        .max()
}

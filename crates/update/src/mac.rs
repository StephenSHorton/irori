//! The macOS platform (Go update/mac_darwin.go): replace the whole Irori.app.
//! It shells out to `ditto`, `plutil`, `codesign` and `xattr` exactly as Go
//! does, so the bundle checks are the ones the Go updater runs on the Rust
//! `.app.zip` (surface.md §7.3). The live bundle is passed in (the
//! updater's `running_app` seam, Go `macPlatform(s, app, pid)`), so
//! tests/mac.rs drives a fake Irori.app. Not yet built or run on the Mac.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::macapp::{BUNDLE_ID, app_bundle_root, side_paths, swap_args, translocated};
use crate::version::numeric_version;

/// Go `runningApp`: the Irori.app this process runs from, when it can be
/// updated in place.
pub fn running_app() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = std::fs::canonicalize(&exe).map_err(|e| e.to_string())?;
    let Some(app) = app_bundle_root(&exe) else {
        return Err("Irori is not running from Irori.app, so it can't update itself; install the new version from the release page".into());
    };
    if translocated(&app) {
        return Err("macOS is running Irori from a temporary read-only copy: move Irori.app into Applications and open it from there to get updates".into());
    }
    let dir = app.parent().unwrap_or(Path::new("/"));
    if let Err(e) = access_w(dir) {
        return Err(format!(
            "can't update Irori: {} is not writable for this user ({e}); install the new version from the release page",
            dir.display()
        ));
    }
    Ok(app)
}

fn access_w(dir: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    // SAFETY: a valid NUL-terminated path.
    if unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

fn combined(cmd: &mut Command) -> Result<(), String> {
    match cmd.output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            let mut out = o.stdout;
            out.extend_from_slice(&o.stderr);
            Err(format!(
                "{}: {}",
                o.status,
                String::from_utf8_lossy(&out).trim()
            ))
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Go `stage`: unpack the verified `.app.zip` with `ditto`, check the one
/// bundle inside, and copy it beside the live bundle as `.Irori.app.new`.
pub fn stage(live: &Path, dir: &Path, zip: &Path, version: &str) -> Result<PathBuf, String> {
    let unpack = dir.join("unpacked");
    let _ = std::fs::remove_dir_all(&unpack);
    let result = (|| {
        combined(
            Command::new("/usr/bin/ditto")
                .arg("-x")
                .arg("-k")
                .arg(zip)
                .arg(&unpack),
        )
        .map_err(|e| {
            format!(
                "unpack {}: {e}",
                zip.file_name().unwrap_or_default().to_string_lossy()
            )
        })?;
        let staged = only_app(&unpack)?;
        check_bundle(&staged, version)?;
        check_signature(live, &staged)?;
        let _ = Command::new("/usr/bin/xattr")
            .args(["-dr", "com.apple.quarantine"])
            .arg(&staged)
            .status();
        let (next, _) = side_paths(live);
        let _ = std::fs::remove_dir_all(&next);
        if let Err(e) = combined(Command::new("/usr/bin/ditto").arg(&staged).arg(&next)) {
            let _ = std::fs::remove_dir_all(&next);
            return Err(format!("stage the update next to {}: {e}", live.display()));
        }
        Ok(next)
    })();
    let _ = std::fs::remove_dir_all(&unpack);
    result
}

/// Go `onlyApp`.
fn only_app(dir: &Path) -> Result<PathBuf, String> {
    let mut apps = Vec::new();
    for e in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        if e.file_type().is_ok_and(|t| t.is_dir())
            && e.file_name().to_string_lossy().ends_with(".app")
        {
            apps.push(dir.join(e.file_name()));
        }
    }
    if apps.len() != 1 {
        return Err(format!(
            "update zip holds {} .app bundles, want 1",
            apps.len()
        ));
    }
    Ok(apps.remove(0))
}

/// Go `checkBundle`.
pub fn check_bundle(app: &Path, version: &str) -> Result<(), String> {
    let plist = app.join("Contents").join("Info.plist");
    let id = plist_string(&plist, "CFBundleIdentifier")?;
    if id != BUNDLE_ID {
        return Err(format!("update bundle id is {id:?}, want {BUNDLE_ID:?}"));
    }
    let v = plist_string(&plist, "CFBundleShortVersionString")?;
    let want = numeric_version(version);
    if v != want {
        return Err(format!("update bundle is version {v:?}, want {want:?}"));
    }
    let exe = plist_string(&plist, "CFBundleExecutable")?;
    use std::os::unix::fs::PermissionsExt;
    let ok = std::fs::metadata(app.join("Contents").join("MacOS").join(&exe))
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if !ok {
        return Err(format!(
            "update bundle has no executable Contents/MacOS/{exe}"
        ));
    }
    Ok(())
}

fn plist_string(plist: &Path, key: &str) -> Result<String, String> {
    match Command::new("/usr/bin/plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(plist)
        .output()
    {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).trim().to_string()),
        Ok(o) => Err(format!("read {key} from {}: {}", plist.display(), o.status)),
        Err(e) => Err(format!("read {key} from {}: {e}", plist.display())),
    }
}

/// Go `checkSignature`.
fn check_signature(live: &Path, staged: &Path) -> Result<(), String> {
    combined(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(staged),
    )
    .map_err(|e| format!("update signature does not verify: {e}"))?;
    if developer_id(live) && !developer_id(staged) {
        return Err("this Irori is signed with Developer ID and the update is not; install it from the release page".into());
    }
    Ok(())
}

fn developer_id(path: &Path) -> bool {
    Command::new("/usr/bin/codesign")
        .args(["-dv", "--verbose=2"])
        .arg(path)
        .output()
        .map(|o| {
            let mut s = o.stdout;
            s.extend_from_slice(&o.stderr);
            String::from_utf8_lossy(&s).contains("Authority=Developer ID Application")
        })
        .unwrap_or(false)
}

/// Go `install`: the swap helper's command line, for the launcher to start
/// detached. `live` is the running bundle and `pid` the process the helper
/// waits for.
pub fn install_command(
    live: &Path,
    dir: &Path,
    next: &Path,
    relaunch: bool,
    pid: u32,
) -> Result<(PathBuf, Vec<String>), String> {
    if std::fs::metadata(next.join("Contents").join("Info.plist")).is_err() {
        return Err(format!("staged update is gone: {}", next.display()));
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let log = dir.join("update.log");
    Ok((
        PathBuf::from("/bin/sh"),
        swap_args(pid, live, next, relaunch, &log),
    ))
}

/// Go `cleanup`: a staged bundle an update never installed, and an old one a
/// swap could not delete, beside the live bundle `live`.
pub fn cleanup(live: &Path) {
    let (next, old) = side_paths(live);
    let _ = std::fs::remove_dir_all(next);
    let _ = std::fs::remove_dir_all(old);
}

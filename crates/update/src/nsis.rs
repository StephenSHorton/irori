//! The Windows side (Go update/nsis.go): run the release's NSIS installer
//! silently. Untagged, so its tests run on every OS. The installer is
//! app/nsis/irori.nsi, the port of Go's build/windows/nsis/project.nsi.

use std::path::{Path, PathBuf};

/// Go `waitPIDArg`: the installer waits for this process (the GUI) to exit
/// before it swaps the exe, not for the file lock, which `irori mcp`
/// children can hold for as long as their sessions run.
pub fn wait_pid_arg() -> String {
    format!("/WAITPID={}", std::process::id())
}

/// The installer's arguments: `/S [/NORELAUNCH] /WAITPID=<pid>` (Go
/// nsis.go:16-21).
pub fn install_args(relaunch: bool) -> Vec<String> {
    let mut args = vec!["/S".to_string()];
    if !relaunch {
        args.push("/NORELAUNCH".to_string());
    }
    args.push(wait_pid_arg());
    args
}

/// Go `CleanupOldExes`: deletes the copies of `exe` the installer renamed
/// aside (`<exe>.old`, `.old1`, ...; case-insensitive) while `irori mcp` or
/// `irori serve` still held the old image. A copy still running fails to
/// delete and is left for the next start. Returns the paths it removed.
pub fn cleanup_old_exes(exe: &Path) -> Vec<PathBuf> {
    let Some(dir) = exe.parent() else {
        return Vec::new();
    };
    let Some(name) = exe.file_name() else {
        return Vec::new();
    };
    let prefix = format!("{}.old", name.to_string_lossy().to_lowercase());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for e in entries.flatten() {
        if e.file_type().map(|t| t.is_dir()).unwrap_or(true) {
            continue;
        }
        let n = e.file_name().to_string_lossy().to_lowercase();
        if !is_old_copy(&n, &prefix) {
            continue;
        }
        let p = dir.join(e.file_name());
        if std::fs::remove_file(&p).is_ok() {
            removed.push(p);
        }
    }
    removed
}

/// Go `isOldCopy`: `prefix` itself or `prefix` plus ASCII digits.
fn is_old_copy(name: &str, prefix: &str) -> bool {
    match name.strip_prefix(prefix) {
        Some(rest) => rest.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Go TestWaitPIDArgIsThisProcess.
    #[test]
    fn wait_pid_is_this_process() {
        assert_eq!(wait_pid_arg(), format!("/WAITPID={}", std::process::id()));
        assert_eq!(install_args(true), vec!["/S".to_string(), wait_pid_arg()]);
        assert_eq!(
            install_args(false),
            vec!["/S".to_string(), "/NORELAUNCH".to_string(), wait_pid_arg()]
        );
    }

    // Go TestCleanupOldExes.
    #[test]
    fn cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("irori.exe");
        let keep = [
            "irori.exe",
            "irori.exe.older",
            "irori.exe.old.txt",
            "uninstall.exe",
            "other.exe.old",
        ];
        let drop = ["irori.exe.old", "irori.exe.old1", "IRORI.EXE.OLD12"];
        for n in keep.iter().chain(drop.iter()) {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        let removed = cleanup_old_exes(&exe);
        assert_eq!(removed.len(), drop.len(), "{removed:?}");
        for n in drop {
            assert!(!dir.path().join(n).exists(), "{n} still present");
        }
        for n in keep {
            assert!(dir.path().join(n).exists(), "{n} removed");
        }
    }
}

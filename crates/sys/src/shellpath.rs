//! The login shell's PATH for the desktop app (Go `guipath_darwin.go`).
//!
//! A Mac app opened from Finder, the Dock or a login item gets launchd's
//! PATH (/usr/bin:/bin:/usr/sbin:/sbin), not the one the user's shell sets
//! up. The desktop app starts `claude` (roster's resume) and git, and the
//! agents it starts inherit its environment, so they would miss Homebrew,
//! ~/.local/bin and every version manager. [`fix_gui_path`] asks the login
//! shell for its PATH once at startup, the way editors do, and puts it in
//! front. Elsewhere it does nothing (Go `guipath_other.go`).

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Brackets the PATH in the shell's output, so whatever a login shell
/// prints on start (a motd, a banner) is skipped.
pub const PATH_MARKER: &str = "__IRORI_PATH__";

/// How long the login shell may take.
pub const SHELL_TIMEOUT: Duration = Duration::from_secs(5);

/// How long output may stay open after the shell exits or is killed (Go
/// `cmd.WaitDelay`): a plugin that forks something holding stdout open
/// must not stall startup.
const WAIT_DELAY: Duration = Duration::from_secs(1);

/// Puts the login shell's PATH (or [`fallback_path`]) in front of this
/// process's. macOS only.
///
/// # Safety
///
/// It sets an environment variable: call it before any other thread runs.
pub unsafe fn fix_gui_path() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let cur = std::env::var("PATH").unwrap_or_default();
    let shell_path = login_shell_path();
    let add: Vec<String> = if shell_path.is_empty() {
        fallback_path()
    } else {
        split(&shell_path)
    };
    let merged = merge_path(&add, &split(&cur)).join(":");
    // SAFETY: the caller runs this before any other thread exists.
    unsafe { std::env::set_var("PATH", merged) };
}

fn split(p: &str) -> Vec<String> {
    p.split(':').map(str::to_string).collect()
}

/// Where the Claude Code installer, Homebrew on Apple silicon and Homebrew
/// on Intel put commands, for when the shell can't be asked.
pub fn fallback_path() -> Vec<String> {
    let mut dirs = vec![];
    if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        dirs.push(
            std::path::Path::new(&home)
                .join(".local")
                .join("bin")
                .to_string_lossy()
                .into_owned(),
        );
    }
    dirs.push("/opt/homebrew/bin".into());
    dirs.push("/usr/local/bin".into());
    dirs
}

/// $PATH as the user's login shell sets it, or `""`.
pub fn login_shell_path() -> String {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| std::path::Path::new(s).is_absolute())
        .unwrap_or_else(|| "/bin/zsh".into());
    let script = format!("printf '{PATH_MARKER}%s{PATH_MARKER}' \"$PATH\"");
    let Ok(mut child) = Command::new(shell)
        .arg("-ilc")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return String::new();
    };
    let mut out = child.stdout.take().expect("piped stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    // Wait for the shell, killing it at the timeout.
    let deadline = std::time::Instant::now() + SHELL_TIMEOUT;
    let ok = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st.success(),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    if !ok {
        return String::new();
    }
    match rx.recv_timeout(WAIT_DELAY) {
        Ok(buf) => between(&String::from_utf8_lossy(&buf), PATH_MARKER),
        Err(_) => String::new(),
    }
}

/// The text between the first two markers in `s`, trimmed, or `""`.
pub fn between(s: &str, marker: &str) -> String {
    let Some((_, rest)) = s.split_once(marker) else {
        return String::new();
    };
    let Some((v, _)) = rest.split_once(marker) else {
        return String::new();
    };
    v.trim().to_string()
}

/// `first` followed by the entries of `then` it lacks, without empty
/// entries or repeats.
pub fn merge_path(first: &[String], then: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for d in first.iter().chain(then) {
        if d.is_empty() || out.contains(d) {
            continue;
        }
        out.push(d.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn between_markers() {
        let out = format!(
            "Last login: today\nwelcome banner\n{PATH_MARKER}/opt/homebrew/bin:/usr/bin{PATH_MARKER}\n"
        );
        assert_eq!(between(&out, PATH_MARKER), "/opt/homebrew/bin:/usr/bin");
        assert_eq!(between("no markers", PATH_MARKER), "");
        assert_eq!(between(&format!("{PATH_MARKER}half"), PATH_MARKER), "");
    }

    #[test]
    fn merge() {
        let got = merge_path(&v(&["/a", "", "/b", "/a"]), &v(&["/usr/bin", "/b", "/bin"]));
        assert_eq!(got, v(&["/a", "/b", "/usr/bin", "/bin"]));
    }

    /// The fallback is the macOS one (`/`-joined under $HOME).
    #[cfg(unix)]
    #[test]
    fn fallback_has_homebrew_and_local_bin() {
        let f = fallback_path();
        assert!(f.iter().any(|d| d.ends_with("/.local/bin")), "{f:?}");
        assert!(f.contains(&"/opt/homebrew/bin".to_string()));
        assert!(f.contains(&"/usr/local/bin".to_string()));
    }

    /// This machine's login shell answers with a PATH that has /usr/bin.
    #[cfg(unix)]
    #[test]
    fn login_shell_answers() {
        let p = login_shell_path();
        assert!(p.contains("/usr/bin"), "login shell PATH = {p:?}");
    }
}

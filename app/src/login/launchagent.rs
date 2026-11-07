//! The macOS LaunchAgent fallback (Wails autostart_darwin.go:113-327): a
//! plist in `~/Library/LaunchAgents` for an unbundled exe or a macOS before
//! 13. The plist text and the directory scan are plain file work, so they
//! compile and are tested on every OS; `launchctl` runs only on macOS.

// Only macOS calls it; the tests run it everywhere.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::{Path, PathBuf};

/// Wails `launchAgentPlist`.
pub fn plist(label: &str, exe: &str, args: &[&str]) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
    s.push_str("<plist version=\"1.0\">\n");
    s.push_str("  <dict>\n");
    s.push_str("    <key>Label</key>\n");
    s.push_str(&format!("    <string>{}</string>\n", xml_escape(label)));
    s.push_str("    <key>ProgramArguments</key>\n");
    s.push_str("    <array>\n");
    for a in std::iter::once(exe).chain(args.iter().copied()) {
        s.push_str(&format!("      <string>{}</string>\n", xml_escape(a)));
    }
    s.push_str("    </array>\n");
    s.push_str("    <key>RunAtLoad</key>\n");
    s.push_str("    <true/>\n");
    s.push_str("    <key>KeepAlive</key>\n");
    s.push_str("    <false/>\n");
    s.push_str("  </dict>\n");
    s.push_str("</plist>\n");
    s
}

/// Go `xml.EscapeText`: `"'&<>`, tab, newline and carriage return.
fn xml_escape(s: &str) -> String {
    let mut b = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => b.push_str("&#34;"),
            '\'' => b.push_str("&#39;"),
            '&' => b.push_str("&amp;"),
            '<' => b.push_str("&lt;"),
            '>' => b.push_str("&gt;"),
            '\t' => b.push_str("&#x9;"),
            '\n' => b.push_str("&#xA;"),
            '\r' => b.push_str("&#xD;"),
            c => b.push(c),
        }
    }
    b
}

fn xml_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else { break };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Wails `plistFirstProgramArg`: the first `<string>` of the array after
/// the `ProgramArguments` key, or "" (a malformed file is not ours).
pub fn first_program_arg(data: &str) -> String {
    let Some(k) = data.find("<key>ProgramArguments</key>") else {
        return String::new();
    };
    let rest = data[k + "<key>ProgramArguments</key>".len()..].trim_start();
    let Some(rest) = rest.strip_prefix("<array>") else {
        return String::new();
    };
    let rest = rest.trim_start();
    let Some(rest) = rest.strip_prefix("<string>") else {
        return String::new();
    };
    match rest.find("</string>") {
        Some(end) => xml_unescape(&rest[..end]),
        None => String::new(),
    }
}

/// The LaunchAgents directory: real (`~/Library/LaunchAgents`) or a test's.
pub struct Agents {
    pub dir: PathBuf,
}

impl Agents {
    /// Wails `findLaunchAgent`: the plist whose first program argument is
    /// `exe`.
    pub fn find(&self, exe: &str) -> Result<Option<PathBuf>, String> {
        let rd = match std::fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("autostart: read LaunchAgents dir: {e}")),
        };
        let mut names: Vec<PathBuf> = rd
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "plist"))
            .collect();
        // Go's os.ReadDir sorts by name.
        names.sort();
        for p in names {
            let Ok(data) = std::fs::read_to_string(&p) else {
                continue;
            };
            if first_program_arg(&data) == exe {
                return Ok(Some(p));
            }
        }
        Ok(None)
    }

    /// Wails `enableLaunchAgent`: writes `<label>.plist` atomically.
    /// Returns its path, for `launchctl bootstrap`.
    pub fn enable(&self, label: &str, exe: &str, args: &[&str]) -> Result<PathBuf, String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("autostart: create LaunchAgents dir: {e}"))?;
        let path = self.dir.join(format!("{label}.plist"));
        write_atomic(&path, plist(label, exe, args).as_bytes())
            .map_err(|e| format!("autostart: write plist: {e}"))?;
        Ok(path)
    }

    /// Wails `disableLaunchAgent`: removes the plist pointing at `exe`.
    /// Returns its path when there was one, for `launchctl bootout`
    /// (run before the removal by the caller on macOS).
    pub fn disable(&self, exe: &str, before_remove: impl FnOnce(&Path)) -> Result<(), String> {
        let Some(p) = self.find(exe)? else {
            return Ok(());
        };
        before_remove(&p);
        match std::fs::remove_file(&p) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("autostart: remove plist: {e}")),
        }
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("plist.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_round_trip() {
        let exe = "/Users/a/dev/irori & co/irori";
        let p = plist("Irori", exe, &["--startup"]);
        assert!(p.contains("<string>/Users/a/dev/irori &amp; co/irori</string>"));
        assert!(p.contains("<string>--startup</string>"));
        assert!(p.contains("    <key>RunAtLoad</key>\n    <true/>\n"));
        assert_eq!(first_program_arg(&p), exe);
        assert_eq!(first_program_arg("not a plist"), "");
        assert_eq!(
            first_program_arg("<key>ProgramArguments</key><array></array>"),
            ""
        );
    }

    #[test]
    fn agents_dir_scan() {
        let d = tempfile::tempdir().unwrap();
        let a = Agents {
            dir: d.path().join("LaunchAgents"),
        };
        let exe = "/Applications/Irori.app/Contents/MacOS/irori";
        assert_eq!(a.find(exe).unwrap(), None);
        a.disable(exe, |_| panic!("nothing to boot out")).unwrap();
        let path = a.enable("Irori", exe, &["--startup"]).unwrap();
        assert_eq!(path, a.dir.join("Irori.plist"));
        std::fs::write(a.dir.join("other.plist"), plist("other", "/bin/x", &[])).unwrap();
        assert_eq!(a.find(exe).unwrap(), Some(path.clone()));
        let mut booted = None;
        a.disable(exe, |p| booted = Some(p.to_path_buf())).unwrap();
        assert_eq!(booted, Some(path.clone()));
        assert!(!path.exists());
        assert!(a.dir.join("other.plist").exists());
    }
}

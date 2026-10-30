//! The Windows HKCU Run key, as Wails v3.0.0-alpha.98 writes it
//! (autostart_windows.go:30-199). The command-line helpers compile
//! everywhere so their tables are tested on every OS.
#![cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "only Windows writes the Run key; tests use the helpers everywhere"
    )
)]

/// Wails `defaultAutostartRegistrySubKey`, under HKEY_CURRENT_USER.
pub const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Wails `validateAutostartIdentifier`: at most 200 of `A-Za-z0-9._-`.
pub fn validate_identifier(id: &str) -> Result<(), String> {
    if id.len() > 200 {
        return Err(format!(
            "autostart identifier too long (max 200): {}",
            gocompat::quote::quote(id)
        ));
    }
    for r in id.chars() {
        if !(r.is_ascii_alphanumeric() || r == '.' || r == '_' || r == '-') {
            return Err(format!(
                "autostart identifier contains invalid character {} (allowed: A-Za-z0-9._-)",
                go_quote_rune(r)
            ));
        }
    }
    Ok(())
}

/// Go `%q` of a rune: single quotes, with `strconv.QuoteRune` escapes.
fn go_quote_rune(r: char) -> String {
    let q = gocompat::quote::quote(&r.to_string());
    let inner = &q[1..q.len() - 1];
    let inner = match inner {
        "\\\"" => "\"".to_string(),
        "'" => "\\'".to_string(),
        s => s.to_string(),
    };
    format!("'{inner}'")
}

/// Wails `parseWindowsCommandExe`: the first token of a command line,
/// honouring surrounding double quotes, lowercased.
pub fn command_exe(cmd: &str) -> String {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return String::new();
    }
    if let Some(rest) = cmd.strip_prefix('"') {
        return match rest.find('"') {
            Some(end) => rest[..end].to_lowercase(),
            None => rest.to_lowercase(),
        };
    }
    match cmd.find([' ', '\t']) {
        Some(i) => cmd[..i].to_lowercase(),
        None => cmd.to_lowercase(),
    }
}

/// Wails `quoteWindowsArg`: wraps an argument in double quotes when it is
/// empty or has a space, tab or quote, by CommandLineToArgvW's rules.
pub fn quote_arg(s: &str) -> String {
    if !s.is_empty() && !s.contains([' ', '"', '\t']) {
        return s.to_string();
    }
    let mut b = String::with_capacity(s.len() + 2);
    b.push('"');
    let mut backslashes = 0usize;
    for c in s.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                b.extend(std::iter::repeat_n('\\', 2 * backslashes + 1));
                b.push('"');
                backslashes = 0;
            }
            _ => {
                b.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                b.push(c);
            }
        }
    }
    b.extend(std::iter::repeat_n('\\', 2 * backslashes));
    b.push('"');
    b
}

/// The Run value Wails writes for `exe` and `args`.
pub fn command_line(exe: &str, args: &[&str]) -> String {
    let mut cmd = quote_arg(exe);
    for a in args {
        cmd.push(' ');
        cmd.push_str(&quote_arg(a));
    }
    cmd
}

/// One Run key under HKCU. Production uses [`RUN_SUBKEY`]; tests use a
/// scratch subkey and delete it afterwards.
#[cfg(windows)]
pub struct RunKey {
    subkey: String,
}

#[cfg(windows)]
impl RunKey {
    pub fn new(subkey: &str) -> RunKey {
        RunKey {
            subkey: subkey.to_string(),
        }
    }

    /// Wails `find`: the value name and command whose first token equals
    /// `exe` (case-insensitive). `None` is not registered.
    pub fn find(&self, exe: &str) -> Result<Option<(String, String)>, String> {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE};
        let key = match RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(&self.subkey, KEY_QUERY_VALUE)
        {
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("autostart: open registry key: {e}")),
        };
        let want = exe.to_lowercase();
        for v in key.enum_values() {
            let Ok((name, _)) = v else { continue };
            let Ok(val) = key.get_value::<String, _>(&name) else {
                continue;
            };
            if command_exe(&val) == want {
                return Ok(Some((name, val)));
            }
        }
        Ok(None)
    }

    /// Wails `enable`: writes `id` = the command line, first deleting a
    /// value under another name that points at the same exe.
    pub fn enable(&self, id: &str, exe: &str, args: &[&str]) -> Result<(), String> {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        validate_identifier(id)?;
        let cmd = command_line(exe, args);
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(&self.subkey)
            .map_err(|e| format!("autostart: open registry key: {e}"))?;
        if let Ok(Some((existing, _))) = self.find(exe)
            && existing != id
        {
            let _ = key.delete_value(&existing);
        }
        key.set_value(id, &cmd)
            .map_err(|e| format!("autostart: write registry value: {e}"))
    }

    /// Every `(name, command)` string value under the key; none when the
    /// key is missing.
    pub fn values(&self) -> Result<Vec<(String, String)>, String> {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE};
        let key = match RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(&self.subkey, KEY_QUERY_VALUE)
        {
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(format!("autostart: open registry key: {e}")),
        };
        let mut out = Vec::new();
        for v in key.enum_values() {
            let Ok((name, _)) = v else { continue };
            if let Ok(val) = key.get_value::<String, _>(&name) {
                out.push((name, val));
            }
        }
        Ok(out)
    }

    /// The launch-at-login cutover check (`::update::login::plan`, surface.md
    /// 7.4): a value for `exe` that is not exactly `Irori` = `exe --startup`
    /// in Wails' quoting is rewritten to that. Values for other exes and a
    /// missing registration are left alone. Returns what it did.
    pub fn cutover(&self, exe: &str) -> Result<::update::login::Plan, String> {
        use ::update::login::Plan;
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let plan = ::update::login::plan(&self.values()?, exe, command_exe, command_line);
        if let Plan::Rewrite { command, delete } = &plan {
            let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
                .create_subkey(&self.subkey)
                .map_err(|e| format!("autostart: open registry key: {e}"))?;
            key.set_value(::update::login::ID, command)
                .map_err(|e| format!("autostart: write registry value: {e}"))?;
            if let Some(old) = delete {
                match key.delete_value(old) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("autostart: delete registry value: {e}")),
                }
            }
        }
        Ok(plan)
    }

    /// Wails `disable`: deletes the value that points at `exe`, if any.
    pub fn disable(&self, exe: &str) -> Result<(), String> {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
        let Some((id, _)) = self.find(exe)? else {
            return Ok(());
        };
        let key = match RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(&self.subkey, KEY_SET_VALUE)
        {
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("autostart: open registry key: {e}")),
        };
        match key.delete_value(&id) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("autostart: delete registry value: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Wails TestParseWindowsCommandExe.
    #[test]
    fn command_exe_like_wails() {
        for (input, want) in [
            (r"C:\app\foo.exe", r"c:\app\foo.exe"),
            (
                r#""C:\Program Files\foo\foo.exe""#,
                r"c:\program files\foo\foo.exe",
            ),
            (r"C:\app\foo.exe --flag", r"c:\app\foo.exe"),
            (r#""C:\app\foo.exe" --flag"#, r"c:\app\foo.exe"),
            ("", ""),
            (r"  C:\app\foo.exe", r"c:\app\foo.exe"),
            (r#""C:\unterminated"#, r"c:\unterminated"),
            ("C:\\a.exe\t--startup", r"c:\a.exe"),
        ] {
            assert_eq!(command_exe(input), want, "{input:?}");
        }
    }

    #[test]
    fn quote_like_wails() {
        for (input, want) in [
            (
                r"C:\Users\a\AppData\Local\Programs\Irori\irori.exe",
                r"C:\Users\a\AppData\Local\Programs\Irori\irori.exe",
            ),
            ("--startup", "--startup"),
            ("", r#""""#),
            (
                r"C:\Program Files\Irori\irori.exe",
                r#""C:\Program Files\Irori\irori.exe""#,
            ),
            (r#"a"b"#, r#""a\"b""#),
            (r#"a\"b"#, r#""a\\\"b""#),
            (r"has space\", r#""has space\\""#),
            ("tab\there", "\"tab\there\""),
        ] {
            assert_eq!(quote_arg(input), want, "{input:?}");
        }
        assert_eq!(
            command_line(r"C:\Program Files\Irori\irori.exe", &["--startup"]),
            r#""C:\Program Files\Irori\irori.exe" --startup"#
        );
        // The default per-user install path has no space, so the value
        // the Go build wrote is unquoted.
        assert_eq!(
            command_line(
                r"C:\Users\a\AppData\Local\Programs\Irori\irori.exe",
                &["--startup"]
            ),
            r"C:\Users\a\AppData\Local\Programs\Irori\irori.exe --startup"
        );
    }

    #[test]
    fn identifier_rules() {
        assert!(validate_identifier("Irori").is_ok());
        assert!(validate_identifier("com.example.foo-1_2").is_ok());
        assert!(validate_identifier("").is_ok());
        assert_eq!(
            validate_identifier("bad/identifier").unwrap_err(),
            "autostart identifier contains invalid character '/' (allowed: A-Za-z0-9._-)"
        );
        assert!(validate_identifier(&"a".repeat(201)).is_err());
    }

    /// A scratch HKCU subkey, deleted on drop. The real Run key is never
    /// opened by these tests.
    #[cfg(windows)]
    struct Scratch(String);

    #[cfg(windows)]
    impl Scratch {
        fn new() -> Scratch {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Scratch(format!(
                r"Software\irori-rust-test\{}-{t}\Run",
                std::process::id()
            ))
        }
    }

    #[cfg(windows)]
    impl Drop for Scratch {
        fn drop(&mut self) {
            use winreg::RegKey;
            use winreg::enums::HKEY_CURRENT_USER;
            let parent = self.0.trim_end_matches(r"\Run");
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let _ = hkcu.delete_subkey_all(parent);
            // The shared parent goes too once no other run's key is left
            // (delete_subkey refuses a key with subkeys).
            let _ = hkcu.delete_subkey(r"Software\irori-rust-test");
        }
    }

    #[cfg(windows)]
    #[test]
    fn run_key_round_trip_on_a_scratch_key() {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let s = Scratch::new();
        assert!(s.0.starts_with(r"Software\irori-rust-test\"));
        let run = RunKey::new(&s.0);
        let exe = r"C:\Users\a\AppData\Local\Programs\Irori\irori.exe";

        // Missing key: not registered, disable is a no-op.
        assert_eq!(run.find(exe).unwrap(), None);
        run.disable(exe).unwrap();

        run.enable("Irori", exe, &["--startup"]).unwrap();
        let (name, cmd) = run.find(exe).unwrap().unwrap();
        assert_eq!(name, "Irori");
        assert_eq!(cmd, format!("{exe} --startup"));
        // Case-insensitive on the exe.
        assert!(run.find(&exe.to_uppercase()).unwrap().is_some());
        assert!(run.find(r"C:\other\irori.exe").unwrap().is_none());

        run.disable(exe).unwrap();
        assert_eq!(run.find(exe).unwrap(), None);

        // A stale value under another name pointing at this exe (a quoted
        // one, as for a path with spaces) is seen, and replaced on enable.
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(&s.0)
            .unwrap();
        key.set_value("irori-old", &format!("\"{exe}\" --startup"))
            .unwrap();
        key.set_value("Other", &r"C:\other\app.exe".to_string())
            .unwrap();
        assert_eq!(run.find(exe).unwrap().unwrap().0, "irori-old");
        run.enable("Irori", exe, &["--startup"]).unwrap();
        assert!(key.get_value::<String, _>("irori-old").is_err());
        assert_eq!(run.find(exe).unwrap().unwrap().0, "Irori");
        // Another app's value is left alone.
        assert_eq!(
            key.get_value::<String, _>("Other").unwrap(),
            r"C:\other\app.exe"
        );
        assert!(run.enable("bad/id", exe, &[]).is_err());
    }

    /// The planner with the real Wails parse and quoting (::update::login's
    /// own tests use stand-ins).
    #[test]
    fn cutover_plan_with_wails_rules() {
        use ::update::login::{Plan, plan};
        let v = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        let exe = r"C:\Users\a\AppData\Local\Programs\Irori\irori.exe";
        // What the Go v0.5.0 install wrote: kept.
        let go = v(&[("Irori", &format!("{exe} --startup"))]);
        assert_eq!(plan(&go, exe, command_exe, command_line), Plan::Keep);
        // A path with a tab is quoted, as Wails does (not only on spaces),
        // so its quoted value is already the Rust form.
        let tabbed = "C:\\a\tb\\irori.exe";
        let vals = v(&[("Irori", &format!("\"{tabbed}\" --startup"))]);
        assert_eq!(plan(&vals, tabbed, command_exe, command_line), Plan::Keep);
        let spaced = r"C:\Program Files\Irori\irori.exe";
        let vals = v(&[("Irori", &format!("\"{spaced}\" --startup"))]);
        assert_eq!(plan(&vals, spaced, command_exe, command_line), Plan::Keep);
    }

    #[cfg(windows)]
    #[test]
    fn cutover_on_a_scratch_key() {
        use ::update::login::Plan;
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let s = Scratch::new();
        let run = RunKey::new(&s.0);
        let exe = r"C:\Users\a\AppData\Local\Programs\Irori\irori.exe";

        // Nothing registered (no key at all): nothing written.
        assert_eq!(run.cutover(exe).unwrap(), Plan::Keep);
        assert!(run.values().unwrap().is_empty());

        // An old name for this exe, without --startup: renamed and fixed.
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(&s.0)
            .unwrap();
        key.set_value("irori-old", &format!("\"{exe}\"")).unwrap();
        key.set_value("Other", &r"C:\other\app.exe".to_string())
            .unwrap();
        let dev = r"C:\Users\a\projects\irori\target\debug\irori.exe";
        assert!(matches!(run.cutover(exe).unwrap(), Plan::Rewrite { .. }));
        assert!(key.get_value::<String, _>("irori-old").is_err());
        assert_eq!(
            key.get_value::<String, _>("Irori").unwrap(),
            format!("{exe} --startup")
        );
        assert_eq!(
            key.get_value::<String, _>("Other").unwrap(),
            r"C:\other\app.exe"
        );
        // Already in the Rust form: kept. Another exe (a dev build) does
        // not touch this exe's value.
        assert_eq!(run.cutover(exe).unwrap(), Plan::Keep);
        assert_eq!(run.cutover(dev).unwrap(), Plan::Keep);
        assert_eq!(
            key.get_value::<String, _>("Irori").unwrap(),
            format!("{exe} --startup")
        );
    }
}

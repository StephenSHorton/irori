//! Launch at login across the cutover (architecture.md D9, surface.md §7.4).
//!
//! The Rust installer writes the same `%LOCALAPPDATA%\Programs\Irori\irori.exe`
//! the Go one did, and the `.app` swap keeps `/Applications/Irori.app`, so a
//! registration the Go build made (Wails: HKCU `Run\Irori` = `<exe> --startup`,
//! SMAppService, or `~/Library/LaunchAgents/Irori.plist`) already points at the
//! new build and the M6 shell's `shell::login` reads it as on. What is left is
//! a check at GUI start that brings a registration for *this* exe to the exact
//! form the Rust build writes, so later toggles find and replace it:
//!
//! - registered under a name other than `Irori`, or without `--startup`, or
//!   quoted differently: rewrite it as `Irori` = `command_line(exe, ["--startup"])`
//!   and delete the other name;
//! - registered in that form already, or not registered: nothing.
//!
//! A value pointing at another exe (a dev build, an older install folder) is
//! left alone, as Go does: the person moves it by toggling from the copy they
//! want (Go releasing.md "What the installer does").
//!
//! The planner is pure so it runs on every OS. The Run-value parsing and
//! quoting are the shell's (`shell::login::winrun::{command_exe,
//! command_line}`), passed in so there is one copy of Wails' rules. The GUI
//! start runs it through `shell::login::LoginItem::cutover` (Windows).

/// The Wails Run value name.
pub const ID: &str = "Irori";
/// The login-launch argument.
pub const STARTUP_ARG: &str = "--startup";

/// What the cutover check does to the Run key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Leave the key as it is.
    Keep,
    /// Write `ID` = `command`, then delete `delete` (another name that
    /// pointed at this exe) when set.
    Rewrite {
        command: String,
        delete: Option<String>,
    },
}

/// Plans the check over the Run key's `(name, command)` values for `exe`.
/// `command_exe` is Wails' first-token parse (lowercased) and
/// `command_line` its quoting.
pub fn plan(
    values: &[(String, String)],
    exe: &str,
    command_exe: impl Fn(&str) -> String,
    command_line: impl Fn(&str, &[&str]) -> String,
) -> Plan {
    let want_exe = exe.to_lowercase();
    let Some((name, cmd)) = values.iter().find(|(_, c)| command_exe(c) == want_exe) else {
        return Plan::Keep;
    };
    let canonical = command_line(exe, &[STARTUP_ARG]);
    if name == ID && cmd == &canonical {
        return Plan::Keep;
    }
    Plan::Rewrite {
        command: canonical,
        delete: (name != ID).then(|| name.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Stand-ins with Wails' behaviour for the shapes below (the real ones are
    // shell::login::winrun's, tested against Wails' table there; its
    // `cutover_plan_with_wails_rules` and `cutover_on_a_scratch_key` run
    // this planner with them).
    fn first(cmd: &str) -> String {
        let cmd = cmd.trim();
        if let Some(r) = cmd.strip_prefix('"') {
            return r.split('"').next().unwrap_or("").to_lowercase();
        }
        cmd.split([' ', '\t']).next().unwrap_or("").to_lowercase()
    }
    // Wails quotes an empty argument or one with a space, tab or quote
    // (these shapes have no quotes or trailing backslashes to escape).
    fn line(exe: &str, args: &[&str]) -> String {
        let q = |s: &str| {
            if s.is_empty() || s.contains([' ', '\t', '"']) {
                format!("\"{s}\"")
            } else {
                s.to_string()
            }
        };
        let mut c = q(exe);
        for a in args {
            c.push(' ');
            c.push_str(&q(a));
        }
        c
    }
    fn v(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    const EXE: &str = r"C:\Users\p\AppData\Local\Programs\Irori\irori.exe";

    #[test]
    fn go_value_is_kept() {
        // What Wails wrote for the Go v0.5.0 install: unquoted, --startup.
        let vals = v(&[
            ("OneDrive", r#""C:\x\OneDrive.exe" /background"#),
            ("Irori", &format!("{EXE} --startup")),
        ]);
        assert_eq!(plan(&vals, EXE, first, line), Plan::Keep);
        // Path case differs: still this exe, but rewritten to the exact form.
        let upper = EXE.to_uppercase();
        let vals = v(&[("Irori", &format!("{upper} --startup"))]);
        assert_eq!(
            plan(&vals, EXE, first, line),
            Plan::Rewrite {
                command: format!("{EXE} --startup"),
                delete: None
            }
        );
    }

    #[test]
    fn healed_forms() {
        let vals = v(&[("Irori", &format!("\"{EXE}\""))]);
        assert_eq!(
            plan(&vals, EXE, first, line),
            Plan::Rewrite {
                command: format!("{EXE} --startup"),
                delete: None
            }
        );
        let vals = v(&[("irori-old", &format!("{EXE} --startup"))]);
        assert_eq!(
            plan(&vals, EXE, first, line),
            Plan::Rewrite {
                command: format!("{EXE} --startup"),
                delete: Some("irori-old".into())
            }
        );
    }

    #[test]
    fn other_exes_untouched() {
        let vals = v(&[(
            "Irori",
            r"C:\Users\p\projects\irori\bin\irori.exe --startup",
        )]);
        assert_eq!(plan(&vals, EXE, first, line), Plan::Keep);
        assert_eq!(plan(&[], EXE, first, line), Plan::Keep);
        let spaced = r"C:\Program Files\Irori\irori.exe";
        let vals = v(&[("Irori", &format!("\"{spaced}\" --startup"))]);
        assert_eq!(plan(&vals, spaced, first, line), Plan::Keep);
    }
}

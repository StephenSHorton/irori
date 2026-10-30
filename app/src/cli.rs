//! The one binary's role dispatch (Go `main`, I:main.go:39-72;
//! architecture.md 4.1, surface.md 1): the first argument names the role
//! only when it is non-empty and does not start with `-`, so `irori
//! --startup` (the login launch) is the window with `["--startup"]`.

/// Every role Go knows, in the order its unknown-command error lists them.
pub const ROLES: &str = "gui, serve, mcp, attach, events, pair, node, version";

/// Go's split of `os.Args[1:]` into the role and its arguments.
pub fn split(args: &[String]) -> (&str, &[String]) {
    match args.first() {
        Some(a) if !a.is_empty() && !a.starts_with('-') => (a.as_str(), &args[1..]),
        _ => ("", args),
    }
}

/// Go's error for a role it does not know (main.go:66).
pub fn unknown(cmd: &str) -> String {
    format!("unknown command {} ({ROLES})", gocompat::quote::quote(cmd))
}

/// Go `wantsConsole` (console.go): the command-line roles print for a
/// person at a prompt; the window never wants a console, and `irori mcp`
/// speaks only on the stdio pipes its agent gave it.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only the Windows exe attaches a console")
)]
pub fn wants_console(cmd: &str) -> bool {
    !matches!(cmd, "" | "gui" | "mcp")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn role_split_like_go() {
        let a = v(&[]);
        assert_eq!(split(&a), ("", &a[..]));
        let a = v(&["--startup"]);
        assert_eq!(split(&a), ("", &a[..]));
        let a = v(&["-startup", "x"]);
        assert_eq!(split(&a), ("", &a[..]));
        let a = v(&["gui", "--startup"]);
        assert_eq!(split(&a), ("gui", &a[1..]));
        let a = v(&["serve", "--headless"]);
        assert_eq!(split(&a), ("serve", &a[1..]));
        // An empty first argument is no role, as in Go.
        let a = v(&["", "serve"]);
        assert_eq!(split(&a), ("", &a[..]));
    }

    #[test]
    fn unknown_command_text() {
        assert_eq!(
            unknown("bogus"),
            "unknown command \"bogus\" (gui, serve, mcp, attach, events, pair, node, version)"
        );
        assert_eq!(
            unknown("a\tb"),
            "unknown command \"a\\tb\" (gui, serve, mcp, attach, events, pair, node, version)"
        );
    }

    #[test]
    fn console_roles() {
        for c in ["", "gui", "mcp"] {
            assert!(!wants_console(c), "{c}");
        }
        for c in [
            "version", "serve", "attach", "events", "pair", "node", "bogus",
        ] {
            assert!(wants_console(c), "{c}");
        }
    }
}

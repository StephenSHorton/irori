//! `irori node install` and `irori node uninstall` (Go node_cmd.go): the
//! headless fallback for the always-on node. Install registers a Task
//! Scheduler entry that runs `irori serve --headless` at logon and shows
//! the exact command first. The recommended node is the desktop app
//! minimised to the tray, which keeps the updater; a headless node is
//! updated by hand. Pair it first, from the desktop app or with `irori pair
//! join`.

/// The Task Scheduler entry `irori node install` writes.
pub const NODE_TASK: &str = "Irori node";

pub fn run(args: &[String]) -> Result<(), String> {
    if args.len() != 1 || (args[0] != "install" && args[0] != "uninstall") {
        return Err("usage: irori node install | irori node uninstall".into());
    }
    if !cfg!(windows) {
        return Err("irori node install is for Windows (Task Scheduler); on this OS run `irori serve --headless` from your own login item, or use the desktop app".into());
    }
    let install = args[0] == "install";
    let argv = if install {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        install_args(&exe.to_string_lossy())
    } else {
        uninstall_args()
    };
    println!("Running: {}", quote_args(&argv).join(" "));
    let status = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "schtasks: exec: \"{}\": executable file not found in %PATH%",
                    argv[0]
                )
            } else {
                format!("schtasks: {e}")
            }
        })?;
    if !status.success() {
        return Err(match status.code() {
            Some(c) => format!("schtasks: exit status {c}"),
            None => "schtasks: signal: killed".into(),
        });
    }
    if install {
        println!(
            "Irori will run headless at your next logon. Updates to a headless node are by hand."
        );
    }
    Ok(())
}

/// The schtasks command that runs `exe` headless at logon, for this user,
/// without raised rights. `/F` replaces an earlier entry, so running
/// install again after moving irori.exe points it at the new path.
pub fn install_args(exe: &str) -> Vec<String> {
    let mut v: Vec<String> = [
        "schtasks", "/Create", "/TN", NODE_TASK, "/SC", "ONLOGON", "/RL", "LIMITED", "/F", "/TR",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    v.push(format!("\"{exe}\" serve --headless"));
    v
}

pub fn uninstall_args() -> Vec<String> {
    ["schtasks", "/Delete", "/TN", NODE_TASK, "/F"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Quotes the arguments with spaces or quotes for display, the way cmd.exe
/// would take them.
pub fn quote_args(argv: &[String]) -> Vec<String> {
    argv.iter()
        .map(|a| {
            if a.contains([' ', '"']) {
                format!("\"{}\"", a.replace('"', "\\\""))
            } else {
                a.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Go TestNodeInstallArgs: install registers `irori serve --headless`
    /// at logon with the exe path quoted, and shows the command it runs.
    /// Nothing here runs schtasks.
    #[test]
    fn node_install_args() {
        let exe = r"C:\Users\me\AppData\Local\Programs\Irori\irori.exe";
        let got = install_args(exe);
        let want = [
            "schtasks",
            "/Create",
            "/TN",
            "Irori node",
            "/SC",
            "ONLOGON",
            "/RL",
            "LIMITED",
            "/F",
            "/TR",
            r#""C:\Users\me\AppData\Local\Programs\Irori\irori.exe" serve --headless"#,
        ];
        assert_eq!(got, want);
        let shown = quote_args(&got).join(" ");
        assert!(shown.contains(r#"/TN "Irori node""#), "{shown}");
        assert!(
            shown.contains(
                r#"/TR "\"C:\Users\me\AppData\Local\Programs\Irori\irori.exe\" serve --headless""#
            ),
            "{shown}"
        );
        assert_eq!(
            uninstall_args(),
            ["schtasks", "/Delete", "/TN", "Irori node", "/F"]
        );
        let e = run(&["start".to_string()]).unwrap_err();
        assert!(e.contains("usage"), "{e}");
        assert_eq!(e, "usage: irori node install | irori node uninstall");
        assert!(run(&[]).is_err());
        assert!(run(&["install".into(), "x".into()]).is_err());
    }
}

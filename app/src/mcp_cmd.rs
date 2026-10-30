//! `irori mcp` (I:mcp.go): the stdio MCP server until the client closes
//! the pipe.
//!
//! stdout is the protocol channel, so everything else goes to stderr. The
//! Windows release is a GUI-subsystem exe with no console; that is fine,
//! the client spawns it with pipes for stdin and stdout, and this role
//! never attaches a console (surface.md 7.15).
//!
//! The Core is dialed on demand: Irori may start (or restart on a new
//! port) after the client spawned us. A dial reads the data dir's
//! `irori.json` and checks `/v1/health` (Go `api.Dial`). A dev build
//! without `IRORI_HOME` has no data dir, so every tool answers "not
//! running" rather than read the live one.

use std::sync::Arc;

use mcp::{Core, HttpCore, Options, Server};

/// Go `flag.Parse` on a FlagSet with no flags: `-h`/`-help` asks for
/// usage, any other flag is undefined, the first non-flag ends parsing.
fn parse_flags(args: &[String]) -> Result<(), String> {
    // Only the first argument matters: a non-flag ends parsing, and any
    // flag is an error.
    if let Some(a) = args.first() {
        if a == "--" || !a.starts_with('-') || a == "-" {
            return Ok(());
        }
        let name = a.trim_start_matches('-');
        let name = name.split('=').next().unwrap_or("");
        if name == "h" || name == "help" {
            eprintln!("Usage of mcp:");
            return Err("flag: help requested".into());
        }
        eprintln!("flag provided but not defined: -{name}\nUsage of mcp:");
        return Err(format!("flag provided but not defined: -{name}"));
    }
    Ok(())
}

/// Go's slog text handler at warn level, on stderr.
fn log_line(msg: &str) {
    eprintln!("level=WARN msg={}", gocompat::quote::quote(msg));
}

/// Reads the lockfile and probes the Core it names.
fn dial() -> Result<Arc<dyn Core>, String> {
    let dir = crate::datadir::process_data_dir()?;
    let lf = irori_core::lockfile::read(&dir).map_err(|e| e.to_string())?;
    let c = HttpCore::new(&mcp::loopback_url(lf.port), &lf.token);
    match c.health() {
        Ok((true, dev)) if lf.device_id.is_empty() || dev == lf.device_id => Ok(Arc::new(c)),
        Ok(_) => Err(format!(
            "irori is not running: port {} is not the irori in {}",
            lf.port,
            dir.display()
        )),
        Err(e @ mcp::CoreError::Status { .. }) => Err(e.to_string()),
        Err(e) => Err(format!(
            "irori is not running: no answer on port {}: {e}",
            lf.port
        )),
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    parse_flags(args)?;
    let srv = Server::new(Options {
        dial: Some(Arc::new(dial)),
        version: crate::commands::version().to_string(),
        log: Some(Arc::new(log_line)),
        getenv: Some(Arc::new(|k: &str| std::env::var(k).unwrap_or_default())),
        ..Options::default()
    });
    let stdin = std::io::stdin();
    srv.run(stdin.lock(), std::io::stdout())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn flags_as_go_parses_them() {
        assert!(parse_flags(&s(&[])).is_ok());
        assert!(parse_flags(&s(&["extra", "-x"])).is_ok());
        assert_eq!(
            parse_flags(&s(&["--verbose=1"])),
            Err("flag provided but not defined: -verbose".into())
        );
        assert_eq!(parse_flags(&s(&["-h"])), Err("flag: help requested".into()));
    }
}

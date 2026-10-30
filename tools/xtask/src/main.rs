//! Local tasks (architecture.md 7.1). There are no CI checks; these run
//! from scripts/check.sh and by hand.
//!
//! - `cargo xtask bindings` writes `frontend/src/gen/bindings.ts` from the
//!   window's commands and types (crates/window, D10).
//! - `cargo xtask bindings --check` fails when that file is not what the
//!   Rust side generates now.

use std::path::PathBuf;
use std::process::ExitCode;

const BINDINGS: &str = "frontend/src/gen/bindings.ts";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["bindings"] => write(),
        ["bindings", "--check"] => check(),
        _ => Err("usage: cargo xtask bindings [--check]".into()),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The generated text with LF line ends, so the check reads the same on
/// every OS whatever git's checkout does.
fn generated() -> Result<String, String> {
    Ok(irori_window::bindings_text()?.replace("\r\n", "\n"))
}

fn write() -> Result<(), String> {
    let path = root().join(BINDINGS);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, generated()?).map_err(|e| format!("{BINDINGS}: {e}"))?;
    println!("wrote {BINDINGS}");
    Ok(())
}

fn check() -> Result<(), String> {
    let want = generated()?;
    let have = std::fs::read_to_string(root().join(BINDINGS))
        .map_err(|e| format!("{BINDINGS}: {e}"))?
        .replace("\r\n", "\n");
    if have != want {
        return Err(format!(
            "{BINDINGS} is not what the window's commands generate; run `cargo xtask bindings`"
        ));
    }
    println!("{BINDINGS} is current");
    Ok(())
}

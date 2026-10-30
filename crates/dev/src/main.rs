//! `irori-dev`: checks for the Rust port that run on a copy of a real data
//! dir (architecture.md §6.3, §8 M2). It refuses a live data dir (one
//! holding `irori.json`, or Go's default dir): it opens `irori.db` for
//! writing (open applies revocation cuts, and
//! `--append` writes a row).
//!
//!   irori-dev verify --home <copy> [--go-hashes <tsv>] [--lockfile <irori.json>]
//!                    [--append <manifest>]
//!
//!   irori-dev replay --home <copy> --out <dir> [--now <rfc3339>]
//!   irori-dev replay --compare <dir>
//!
//!   irori-dev probe --irori <exe> --home <copy> --claude <empty dir>
//!                   [--go-tools <fallback_tools.json>]
//!
//! Output is counts and row keys, never payloads, keys or tokens.

mod http;
mod probe;
mod replay;
mod verify;

use std::process::ExitCode;

const USAGE: &str = "usage: irori-dev verify --home <copy of a data dir> [--go-hashes <tsv>] [--lockfile <irori.json>] [--append <manifest>]
       irori-dev replay --home <copy of a data dir> --out <dir> [--now <rfc3339>]
       irori-dev replay --compare <dir>
       irori-dev probe --irori <exe> --home <copy of a data dir> --claude <empty dir> [--go-tools <fallback_tools.json>] [--go-lockfile <copy of Go's irori.json>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("verify") => match verify::Opts::parse(&args[1..]) {
            Ok(o) => verify::run(&o),
            Err(e) => {
                eprintln!("irori-dev: {e}\n{USAGE}");
                ExitCode::from(2)
            }
        },
        Some("replay") => match replay::Opts::parse(&args[1..]) {
            Ok(o) => replay::run(&o),
            Err(e) => {
                eprintln!(
                    "irori-dev: {e}
{USAGE}"
                );
                ExitCode::from(2)
            }
        },
        Some("probe") => match probe::Opts::parse(&args[1..]) {
            Ok(o) => probe::run(&o),
            Err(e) => {
                eprintln!(
                    "irori-dev: {e}
{USAGE}"
                );
                ExitCode::from(2)
            }
        },
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

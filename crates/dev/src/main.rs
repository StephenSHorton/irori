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
//!   irori-dev agent-check --home <empty dir> --claude <empty dir> --work <empty dir>
//!                         --irori <exe> --mod <copy of mod/> [--claude-bin <claude>]
//!                         [--model <m>] [--turn-wait <secs>] [--only 1,2,3]
//!       what a real Claude agent does with Irori's tools (agent_check.rs;
//!       scripts/agent-check.sh). Spends API tokens.
//!
//!   irori-dev boards-live host|guest --home <empty dir> --claude <empty dir> ...
//!       one side of the live two-person shared-board run (boards_live.rs;
//!       tools/rehearse/boards-live.sh and .ps1). It makes a fresh person,
//!       so it refuses a --home that holds anything.
//!
//!   irori-dev migrate-spaces --home <copy>
//!       moves the copy's boards/ logs to spaces/ as a start of an Irori
//!       with spaces does (card #392), so the move can be checked on real
//!       data first.
//!
//!   irori-dev space-to-personal --home <copy> --claude <empty dir> --space <id> ...
//!                               [--allow-missing-files]
//!       card #640: moves each one-person space into Personal on the copy,
//!       offline, and checks every card, comment, step and ref
//!       (space_move.rs).
//!
//!   irori-dev spawn-fold --home <copy> --person <id> --root <device> --out <dir> --now <rfc3339>
//!   irori-dev compact-spawn --home <copy> --floor <device>=<seq|top> ... [--dry-run] [--no-vacuum]
//!       card #467: time the Board fold, and delete the superseded
//!       task.spawn rows of v0.11.0's spawn loop (spawn_rows.rs).
//!
//!   irori-dev pair-peer --home <empty dir> --claude <empty dir> [--port <home server port>]
//!       card #680: a scratch Irori with a few cards and an open Add window,
//!       for the Rune phone prototype to pair with (pair_peer.rs).
//!
//! Output is counts and row keys, never payloads, keys or tokens.

mod agent_check;
mod boards_live;
mod http;
mod pair_peer;
mod probe;
mod replay;
mod space_move;
mod spawn_rows;
mod verify;

use std::process::ExitCode;

const USAGE: &str = "usage: irori-dev verify --home <copy of a data dir> [--go-hashes <tsv>] [--lockfile <irori.json>] [--append <manifest>]
       irori-dev replay --home <copy of a data dir> --out <dir> [--now <rfc3339>]
       irori-dev replay --compare <dir>
       irori-dev probe --irori <exe> --home <copy of a data dir> --claude <empty dir> [--go-tools <fallback_tools.json>] [--go-lockfile <copy of Go's irori.json>]
       irori-dev agent-check --home <empty dir> --claude <empty dir> --work <empty dir> --irori <exe> --mod <copy of mod/> [--claude-bin <claude>] [--model <m>] [--turn-wait <secs>] [--only 1,2,3]
       irori-dev boards-live host --home <empty dir> --claude <empty dir> [--link-out <file>] [--step-wait <secs>]
       irori-dev boards-live guest --home <empty dir> --claude <empty dir> --link <irori-invite:...> [--step-wait <secs>]
       irori-dev pair-peer --home <empty dir> --claude <empty dir> [--port <home server port>]
       irori-dev migrate-spaces --home <copy of a data dir>
       irori-dev space-to-personal --home <copy of a data dir> --claude <empty dir> --space <id> ... [--allow-missing-files]
       irori-dev spawn-fold --home <copy of a data dir> --person <personId> --root <rootDeviceId> --out <dir> --now <rfc3339>
       irori-dev compact-spawn --home <copy of a data dir> --floor <device>=<seq|top> ... [--dry-run] [--no-vacuum]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("migrate-spaces") => match args.get(1..3) {
            Some([flag, home]) if flag == "--home" => {
                let home = std::path::Path::new(home);
                if home.join("irori.json").exists() {
                    eprintln!(
                        "irori-dev: {} holds irori.json: a live data dir, not a copy",
                        home.display()
                    );
                    return ExitCode::from(2);
                }
                match irori_core::migrate_board_files(home) {
                    Ok(n) => {
                        println!("moved {n} board log(s) to spaces/");
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("irori-dev: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
            _ => {
                eprintln!("irori-dev: migrate-spaces needs --home <copy>\n{USAGE}");
                ExitCode::from(2)
            }
        },
        Some("space-to-personal") => match space_move::Opts::parse(&args[1..]) {
            Ok(o) => space_move::run(&o),
            Err(e) => {
                eprintln!("irori-dev: {e}\n{USAGE}");
                ExitCode::from(2)
            }
        },
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
        Some("agent-check") => match agent_check::Opts::parse(&args[1..]) {
            Ok(o) => agent_check::run(&o),
            Err(e) => {
                eprintln!("irori-dev: {e}\n{USAGE}");
                ExitCode::from(2)
            }
        },
        Some("boards-live") => match boards_live::Opts::parse(&args[1..]) {
            Ok(o) => boards_live::run(&o),
            Err(e) => {
                eprintln!("irori-dev: {e}\n{USAGE}");
                ExitCode::from(2)
            }
        },
        Some("pair-peer") => match pair_peer::Opts::parse(&args[1..]) {
            Ok(o) => pair_peer::run(&o),
            Err(e) => {
                eprintln!("irori-dev: {e}\n{USAGE}");
                ExitCode::from(2)
            }
        },
        Some("spawn-fold") => match spawn_rows::FoldOpts::parse(&args[1..]) {
            Ok(o) => spawn_rows::fold(&o),
            Err(e) => {
                eprintln!("irori-dev: {e}\n{USAGE}");
                ExitCode::from(2)
            }
        },
        Some("reconnect-sim") => match spawn_rows::SimOpts::parse(&args[1..]) {
            Ok(o) => spawn_rows::sim(&o),
            Err(e) => {
                eprintln!("irori-dev: {e}\n{USAGE}");
                ExitCode::from(2)
            }
        },
        Some("compact-spawn") => match spawn_rows::CompactOpts::parse(&args[1..]) {
            Ok(o) => spawn_rows::compact(&o),
            Err(e) => {
                eprintln!("irori-dev: {e}\n{USAGE}");
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

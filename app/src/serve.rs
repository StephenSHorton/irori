//! `irori serve --headless` (Go `runServe`, main.go; surface.md §1): the
//! Core with no window, its loopback API on 127.0.0.1:0 and the lockfile,
//! until Ctrl+C. M4 has no network: no mesh, no home server.

use crate::commands::version;
use crate::{datadir, host};

/// Go `flag` for serve's one flag: `-headless`, `--headless`,
/// `-headless=true|false`.
fn parse(args: &[String]) -> Result<bool, String> {
    let mut headless = false;
    for a in args {
        if a == "--" || !a.starts_with('-') {
            break;
        }
        let name = a.trim_start_matches('-');
        let (name, val) = match name.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (name, None),
        };
        if name != "headless" {
            return Err(format!("flag provided but not defined: -{name}"));
        }
        headless = match val {
            None | Some("1" | "t" | "T" | "true" | "TRUE" | "True") => true,
            Some("0" | "f" | "F" | "false" | "FALSE" | "False") => false,
            Some(v) => {
                return Err(format!(
                    "invalid boolean value {} for -headless: parse error",
                    gocompat::quote::quote(v)
                ));
            }
        };
    }
    Ok(headless)
}

pub fn run(args: &[String]) -> Result<(), String> {
    if !parse(args)? {
        return Err("serve needs --headless (the desktop app is plain `irori`)".into());
    }
    let data = datadir::process_data_dir()?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let hosted = host::start(&rt, data.clone(), true, None).map_err(|e| e.to_string())?;
    irori_core::log(&format!(
        "irori {} serving headless (device {}, data {}){}",
        version(),
        hosted.core.device_id(),
        data.display(),
        if host::offline() { ", offline" } else { "" }
    ));
    rt.block_on(async {
        let _ = tokio::signal::ctrl_c().await;
    });
    hosted.close()
}

#[cfg(test)]
mod tests {
    use super::parse;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_like_go() {
        assert!(parse(&args(&["--headless"])).unwrap());
        assert!(parse(&args(&["-headless"])).unwrap());
        assert!(parse(&args(&["-headless=true"])).unwrap());
        assert!(!parse(&args(&["--headless=false"])).unwrap());
        assert!(!parse(&args(&[])).unwrap());
        assert_eq!(
            parse(&args(&["-x"])).unwrap_err(),
            "flag provided but not defined: -x"
        );
        assert!(parse(&args(&["-headless=maybe"])).is_err());
    }
}

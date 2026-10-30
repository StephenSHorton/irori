//! The macOS updater's plain path and string work (Go update/macapp.go),
//! untagged so its tests run on every OS; mac.rs does the rest.

use std::path::{Path, PathBuf};

/// Go `macBundleID`: Irori.app's `CFBundleIdentifier` (app/macos/Info.plist).
/// An update whose bundle says otherwise is refused.
pub const BUNDLE_ID: &str = "com.stephenshorton.irori";

/// Go `appBundleRoot`: the `.app` bundle `exe` runs from
/// (`.../Irori.app/Contents/MacOS/irori` gives `.../Irori.app`), or `None`.
pub fn app_bundle_root(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    if macos.file_name()? != "MacOS" {
        return None;
    }
    let contents = macos.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let app = contents.parent()?;
    if !app.to_string_lossy().ends_with(".app") {
        return None;
    }
    Some(app.to_path_buf())
}

/// Go `translocated`: App Translocation's read-only copy, where an update
/// would be thrown away.
pub fn translocated(app: &Path) -> bool {
    app.to_string_lossy().contains("/AppTranslocation/")
}

/// Go `sidePaths`: where the new bundle is staged (`.Irori.app.new`) and the
/// old one parked during the swap (`.Irori.app.old`), hidden siblings of the
/// live bundle so both renames stay on one volume.
pub fn side_paths(app: &Path) -> (PathBuf, PathBuf) {
    let dir = app.parent().unwrap_or(Path::new(""));
    let base = app
        .file_name()
        .map(|b| b.to_string_lossy().into_owned())
        .unwrap_or_default();
    (
        dir.join(format!(".{base}.new")),
        dir.join(format!(".{base}.old")),
    )
}

/// Go `swapScript`, byte for byte: the detached helper that finishes a macOS
/// update.
///
/// `sh -c SWAP_SCRIPT irori-update <pid> <live .app> <staged .app> <relaunch 0|1> <log>`
pub const SWAP_SCRIPT: &str = r#"pid=$1 live=$2 next=$3 relaunch=$4
exec >>"$5" 2>&1
old="${live%/*}/.${live##*/}.old"
echo "$(/bin/date): update: waiting for $pid"
n=0
while kill -0 "$pid" 2>/dev/null; do
	n=$((n+1))
	if [ "$n" -gt 600 ]; then
		echo "update: $pid still running after 2 minutes; leaving $live as it is"
		exit 1
	fi
	/bin/sleep 0.2
done
/bin/rm -rf "$old"
if /bin/mv "$live" "$old"; then
	if /bin/mv "$next" "$live"; then
		/bin/rm -rf "$old"
		echo "update: installed $live"
	else
		/bin/mv "$old" "$live"
		echo "update: could not move $next into place; kept the old Irori"
	fi
else
	echo "update: could not move $live aside; kept the old Irori"
fi
if [ "$relaunch" = 1 ]; then
	exec /usr/bin/open "$live"
fi
"#;

/// The swap helper's argv after `/bin/sh` (Go mac_darwin.go:283-300).
pub fn swap_args(pid: u32, live: &Path, next: &Path, relaunch: bool, log: &Path) -> Vec<String> {
    vec![
        "-c".into(),
        SWAP_SCRIPT.into(),
        "irori-update".into(),
        pid.to_string(),
        live.to_string_lossy().into_owned(),
        next.to_string_lossy().into_owned(),
        if relaunch { "1" } else { "0" }.into(),
        log.to_string_lossy().into_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // Go TestAppBundleRoot.
    #[test]
    fn bundle_root() {
        let app = Path::new("Applications").join("Irori.app");
        let cases = [
            (
                app.join("Contents").join("MacOS").join("irori"),
                Some(app.clone()),
            ),
            (Path::new("bin").join("irori"), None),
            (
                Path::new("x").join("Contents").join("MacOS").join("irori"),
                None,
            ),
            (app.join("Contents").join("Resources").join("irori"), None),
        ];
        for (exe, want) in cases {
            assert_eq!(app_bundle_root(&exe), want, "{exe:?}");
        }
        assert!(translocated(Path::new(
            "/private/var/folders/x/T/AppTranslocation/ABC/d/Irori.app"
        )));
        assert!(!translocated(Path::new("/Applications/Irori.app")));
    }

    // Go TestSidePaths.
    #[test]
    fn side() {
        let app = Path::new("Applications").join("Irori.app");
        let (next, old) = side_paths(&app);
        assert_eq!(next, Path::new("Applications").join(".Irori.app.new"));
        assert_eq!(old, Path::new("Applications").join(".Irori.app.old"));
    }

    #[test]
    fn swap_argv() {
        let a = swap_args(
            42,
            Path::new("/Applications/Irori.app"),
            Path::new("/Applications/.Irori.app.new"),
            true,
            Path::new("/u/.irori/updates/update.log"),
        );
        assert_eq!(a[0], "-c");
        assert_eq!(a[2], "irori-update");
        assert_eq!(
            &a[3..],
            [
                "42",
                "/Applications/Irori.app",
                "/Applications/.Irori.app.new",
                "1",
                "/u/.irori/updates/update.log"
            ]
        );
        assert!(
            SWAP_SCRIPT.starts_with("pid=$1 live=$2 next=$3 relaunch=$4\nexec >>\"$5\" 2>&1\n")
        );
        assert!(SWAP_SCRIPT.ends_with("\texec /usr/bin/open \"$live\"\nfi\n"));
    }
}

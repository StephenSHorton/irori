//! Go update/workflow_test.go `TestReleaseWorkflowAssetNames`, over the local
//! release scripts that replace release.yml: the assets they write are the
//! ones each platform's updater picks (this crate's, and the Go v0.5.x one
//! that installs the first Rust release), neither updater takes the other's
//! assets, the portable zip or the dmg, and SHA256SUMS is written. Also
//! checks the installer and bundle templates keep what the Go updater and
//! the existing install rely on.

use update::release::{is_checksum_asset, is_installer_asset, is_mac_app_asset};

fn repo_file(rel: &str) -> String {
    let p = manifest_dir().join("../..").join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// `$<var> = "..."` (PowerShell) or `<var>="..."` (bash) with `$v` → 1.2.3.
fn assigned(text: &str, var: &str, ps: bool) -> String {
    let needle = if ps {
        format!("${var} = \"")
    } else {
        format!("\n{var}=\"")
    };
    let i = text
        .find(&needle)
        .unwrap_or_else(|| panic!("no {var} assignment"));
    let rest = &text[i + needle.len()..];
    rest[..rest.find('"').unwrap()].replace("$v", "1.2.3")
}

#[test]
fn release_script_asset_names() {
    let win = repo_file("scripts/release.ps1");
    let mac = repo_file("scripts/release-mac.sh");
    let installer = assigned(&win, "installer", true);
    let zip = assigned(&win, "zip", true);
    assert_eq!(installer, "irori-1.2.3-windows-amd64-installer.exe");
    assert!(is_installer_asset(&installer));
    assert!(!is_installer_asset(&zip), "the zip {zip} would be taken");

    let appzip = assigned(&mac, "appzip", false);
    let dmg = assigned(&mac, "dmg", false);
    assert_eq!(appzip, "irori-1.2.3-macos-arm64.app.zip");
    assert_eq!(dmg, "irori-1.2.3-macos-arm64.dmg");
    assert!(is_mac_app_asset(&appzip, "arm64"));
    for other in [&dmg, &installer, &zip] {
        assert!(!is_mac_app_asset(other, "arm64"), "mac would take {other}");
    }
    for other in [&appzip, &dmg] {
        assert!(!is_installer_asset(other), "windows would take {other}");
    }

    assert!(win.contains("\"SHA256SUMS\"") && mac.contains("SHA256SUMS"));
    assert!(is_checksum_asset("SHA256SUMS"));
    // The sha256sum line format the updater parses.
    assert!(win.contains("\"$h  $n`n\""));
    assert!(mac.contains("shasum -a 256"));
    // Drafts until both platforms are up; /releases/latest skips them.
    assert!(win.contains("\"--draft\""));
    assert_eq!(update::REPO, "StephenSHorton/irori");
}

#[test]
fn nsis_template_keeps_the_go_contract() {
    let nsi = repo_file("app/nsis/irori.nsi");
    for want in [
        r#"InstallDir "$LOCALAPPDATA\Programs\${INFO_PRODUCTNAME}""#,
        r#"!define UNINST_KEY_NAME "${INFO_COMPANYNAME}${INFO_PRODUCTNAME}""#,
        r#"!define INFO_COMPANYNAME "Stephen Horton""#,
        r#"!define INFO_PRODUCTNAME "Irori""#,
        "RequestExecutionLevel user",
        r#"${GetOptions} $R0 "/WAITPID=" $R2"#,
        r#"${GetOptions} $R0 "/NORELAUNCH" $R1"#,
        "!define IRORI_WAITPID_MS 60000",
        r#"FileOpen $1 "$INSTDIR\${PRODUCT_EXECUTABLE}" a"#,
        r#"Rename "$INSTDIR\${PRODUCT_EXECUTABLE}" "$7""#,
        "IntCmp $6 10 irori_gave_up irori_swap_try irori_gave_up",
        "SetErrorLevel 3",
        r#"DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Irori""#,
        r#"RMDir /r "$AppData\${PRODUCT_EXECUTABLE}""#,
        r#"CreateShortcut "$SMPROGRAMS\${INFO_PRODUCTNAME}.lnk""#,
        r#"${If} $FreshInstall == "1""#,
    ] {
        assert!(nsi.contains(want), "irori.nsi lost: {want}");
    }
    // The Tauri identifier whose WebView2 folder the uninstaller removes.
    let conf = repo_file("app/tauri.conf.json");
    assert!(conf.contains(r#""identifier": "com.stephenshorton.irori""#));
    assert!(nsi.contains(r#"!define TAURI_IDENTIFIER "com.stephenshorton.irori""#));
    assert!(conf.contains(r#""mainBinaryName": "irori""#));
}

#[test]
fn info_plist_passes_the_bundle_checks() {
    let plist = repo_file("app/macos/Info.plist");
    let value = |key: &str| {
        let k = format!("<key>{key}</key>");
        let i = plist.find(&k).unwrap_or_else(|| panic!("no {key}"));
        let rest = &plist[i + k.len()..];
        let s = rest.find("<string>").unwrap() + "<string>".len();
        rest[s..s + rest[s..].find("</string>").unwrap()].to_string()
    };
    assert_eq!(value("CFBundleIdentifier"), update::macapp::BUNDLE_ID);
    assert_eq!(value("CFBundleExecutable"), "irori");
    assert_eq!(value("LSMinimumSystemVersion"), "12.0.0");
    assert_eq!(value("CFBundlePackageType"), "APPL");
    assert_eq!(value("CFBundleIconFile"), "icons");
    assert!(!value("NSLocalNetworkUsageDescription").is_empty());
    let pkg = repo_file("app/macos/package.sh");
    for want in [
        r#"cp "$BINARY" "$APP/Contents/MacOS/irori""#,
        "plutil -replace CFBundleShortVersionString -string \"$NUMERIC\"",
        r#"ditto -c -k --keepParent "Irori.app" "$ZIP""#,
        "codesign --verify --deep --strict",
        r#"bash "$HERE/check-bundle.sh""#,
    ] {
        assert!(pkg.contains(want), "package.sh lost: {want}");
    }
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}

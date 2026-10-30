//! The release side of the contract (Go update/release.go): which assets a
//! release must carry and how `SHA256SUMS` is read. The asset names are also
//! what `scripts/release.ps1` and `scripts/release-mac.sh` write
//! (tests/release_names.rs keeps them in step, as Go's workflow_test.go does).

use serde::Deserialize;

/// A release asset as `releases/latest` lists it.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GhAsset {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "browser_download_url")]
    pub url: String,
}

/// The fields of `releases/latest` the updater reads.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GhRelease {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default, deserialize_with = "null_string")]
    pub body: String,
    #[serde(default, deserialize_with = "null_string")]
    pub published_at: String,
    #[serde(default, deserialize_with = "null_vec")]
    pub assets: Vec<GhAsset>,
}

// Go decodes a JSON null into a string or slice as the zero value.
fn null_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

fn null_vec<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<GhAsset>, D::Error> {
    Ok(Option::<Vec<GhAsset>>::deserialize(d)?.unwrap_or_default())
}

/// Go `isInstallerAsset`: the Windows NSIS installer,
/// `irori-<v>-windows-<arch>-installer.exe`.
pub fn is_installer_asset(name: &str) -> bool {
    let n = name.to_lowercase();
    n.ends_with("-installer.exe") && n.contains("windows")
}

/// Go `isMacAppAsset`: the zipped Irori.app for `arch` (Go `GOARCH` names:
/// `arm64`, `amd64`) or a universal one; never the `.dmg`.
pub fn is_mac_app_asset(name: &str, arch: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with("irori-")
        && (n.ends_with(&format!("-macos-{arch}.app.zip"))
            || n.ends_with("-macos-universal.app.zip"))
}

/// Go `isChecksumAsset`.
pub fn is_checksum_asset(name: &str) -> bool {
    name.eq_ignore_ascii_case("SHA256SUMS")
}

/// This build's architecture in Go's `GOARCH` spelling, which the asset
/// names use.
pub fn goarch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        other => other,
    }
}

/// The hash `SHA256SUMS` lists for `name` (Go `fetchChecksum`'s scan): a
/// line of exactly two whitespace-separated fields, an optional `*` before
/// the file name, matched on its base name; the hash comes back lowercased.
pub fn checksum_for(sums: &str, name: &str) -> Option<String> {
    for line in sums.split('\n') {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() == 2 {
            let file = fields[1].strip_prefix('*').unwrap_or(fields[1]);
            if base(file) == name {
                return Some(fields[0].to_lowercase());
            }
        }
    }
    None
}

/// Go `filepath.Base` for this OS: `.` for an empty path; otherwise
/// trailing separators (`\` and `/` on Windows, `/` elsewhere) are dropped,
/// then a drive volume (`C:`), and the last element is returned, or one
/// separator when nothing is left. UNC volumes are not split off: a
/// `SHA256SUMS` line never names one.
pub fn base(p: &str) -> String {
    let is_sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    if p.is_empty() {
        return ".".into();
    }
    let mut p = p.trim_end_matches(is_sep);
    if cfg!(windows) {
        let b = p.as_bytes();
        if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
            p = &p[2..];
        }
    }
    let last = match p.rfind(is_sep) {
        Some(i) => &p[i + 1..],
        None => p,
    };
    if last.is_empty() {
        return std::path::MAIN_SEPARATOR.to_string();
    }
    last.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Go TestIsMacAppAsset.
    #[test]
    fn mac_asset() {
        let cases = [
            ("irori-1.2.3-macos-arm64.app.zip", "arm64", true),
            ("irori-1.2.3-macos-universal.app.zip", "arm64", true),
            ("irori-1.2.3-macos-universal.app.zip", "amd64", true),
            ("irori-1.2.3-macos-arm64.app.zip", "amd64", false),
            ("irori-1.2.3-macos-arm64.dmg", "arm64", false),
            ("irori-1.2.3-windows-amd64.zip", "amd64", false),
            ("irori-1.2.3-windows-amd64-installer.exe", "amd64", false),
            ("SHA256SUMS", "arm64", false),
        ];
        for (n, a, w) in cases {
            assert_eq!(is_mac_app_asset(n, a), w, "{n} {a}");
        }
        assert!(!is_installer_asset("irori-1.2.3-macos-arm64.app.zip"));
        assert!(is_installer_asset(
            "IRORI-0.6.0-WINDOWS-amd64-Installer.EXE"
        ));
        assert!(is_checksum_asset("sha256sums"));
    }

    #[test]
    fn checksum_lines() {
        let h = "AB".repeat(32);
        let sums = format!(
            "{}  irori-1.2.3-windows-amd64.zip\n{h} *dist/irori-1.2.3-windows-amd64-installer.exe\r\nbad line here x\n",
            "0".repeat(64)
        );
        assert_eq!(
            checksum_for(&sums, "irori-1.2.3-windows-amd64-installer.exe"),
            Some(h.to_lowercase())
        );
        assert_eq!(checksum_for(&sums, "missing"), None);
        assert_eq!(checksum_for("x y z\n", "y"), None);
    }

    #[test]
    fn go_base() {
        assert_eq!(base(""), ".");
        assert_eq!(base("a/b/"), "b");
        assert_eq!(base("///"), std::path::MAIN_SEPARATOR.to_string());
        assert_eq!(base("x"), "x");
        if cfg!(windows) {
            assert_eq!(base(r"a\b.exe"), "b.exe");
            assert_eq!(base("C:"), r"\");
            assert_eq!(base("C:x"), "x");
            assert_eq!(base(r"C:\"), r"\");
        } else {
            assert_eq!(base(r"a\b.exe"), r"a\b.exe");
        }
    }

    #[test]
    fn release_json() {
        let r: GhRelease = serde_json::from_str(
            r#"{"tag_name":"v0.6.0","body":null,"published_at":"2026-10-29T00:00:00Z","assets":[{"name":"SHA256SUMS","browser_download_url":"http://x/s","size":3}],"extra":1}"#,
        )
        .unwrap();
        assert_eq!(r.tag_name, "v0.6.0");
        assert_eq!(r.body, "");
        assert_eq!(r.assets[0].url, "http://x/s");
    }
}

//! The Windows exe's resources (the Tauri build's tauri-build did this):
//! the icon Explorer, the taskbar and the window use (icons/icon.ico),
//! the VERSIONINFO from `IRORI_VERSION` (the release scripts set it; a dev
//! build is 0.0.0.0), and irori.exe.manifest (Common-Controls 6,
//! per-monitor-v2 DPI, asInvoker). Nothing on other targets.

use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rerun-if-changed={}",
        dir.join("icons/icon.ico").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        dir.join("irori.exe.manifest").display()
    );
    println!("cargo:rerun-if-env-changed=IRORI_VERSION");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = std::env::var("IRORI_VERSION").unwrap_or_default();
    let rc = rc_source(dir, &version);
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let path = out.join("irori.rc");
    std::fs::write(&path, rc).expect("write irori.rc");
    windows::compile(&path);
}

#[cfg(windows)]
mod windows {
    pub fn compile(rc: &std::path::Path) {
        embed_resource::compile(rc, embed_resource::NONE)
            .manifest_required()
            .expect("embed the exe's resources");
    }
}

/// A cross build from another host has no resource compiler to call: it
/// says so and goes on without the resources.
#[cfg(not(windows))]
mod windows {
    pub fn compile(_rc: &std::path::Path) {
        println!("cargo:warning=irori.exe gets no icon, version or manifest: build it on Windows");
    }
}

/// "1.2.3" as the four numbers VERSIONINFO wants; anything else is 0.0.0.0.
fn numbers(v: &str) -> [u16; 4] {
    let mut n = [0u16; 4];
    let core = v.split(['-', '+']).next().unwrap_or("");
    for (i, p) in core.split('.').take(4).enumerate() {
        match p.parse() {
            Ok(x) => n[i] = x,
            Err(_) => return [0; 4],
        }
    }
    n
}

/// A string for a .rc file: backslashes doubled, quotes doubled.
fn rc_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\"\"")
}

fn rc_source(dir: &Path, version: &str) -> String {
    let [a, b, c, d] = numbers(version);
    let shown = if version.is_empty() { "dev" } else { version };
    let icon = rc_str(&dir.join("icons/icon.ico").display().to_string());
    let manifest = rc_str(&dir.join("irori.exe.manifest").display().to_string());
    format!(
        r#"#pragma code_page(65001)
1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {a},{b},{c},{d}
PRODUCTVERSION {a},{b},{c},{d}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Irori"
      VALUE "FileVersion", "{shown}"
      VALUE "InternalName", "irori"
      VALUE "OriginalFilename", "irori.exe"
      VALUE "ProductName", "Irori"
      VALUE "ProductVersion", "{shown}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    )
}

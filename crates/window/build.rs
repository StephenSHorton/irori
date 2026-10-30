// This crate's test executables link tauri, so on Windows they need the
// Common-Controls 6 manifest the app embeds (comctl6.manifest).
fn main() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("comctl6.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}

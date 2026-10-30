// xtask links tauri through irori-window, so on Windows it needs the
// Common-Controls 6 manifest (crates/window/comctl6.manifest) to start.
fn main() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/window/comctl6.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}

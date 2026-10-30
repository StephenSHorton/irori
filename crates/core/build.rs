//! Records the toolchain for the diag start line (Go's
//! `runtime.Version()` slot): `rustc 1.95.0 (…)` becomes `rust1.95.0`.

fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let v = std::process::Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.split_whitespace().nth(1).map(str::to_string))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=IRORI_RUSTC_VERSION=rust{v}");
    println!("cargo:rerun-if-env-changed=RUSTC");
}

//! The app's version. The window's commands are crates/window's
//! (`irori_window::builder`), one per Go service method the frontend calls.

/// The version the Go build stamps into `main.version`: `IRORI_VERSION` at
/// build time, else `dev` (surface.md 7.20).
pub fn version() -> &'static str {
    option_env!("IRORI_VERSION").unwrap_or("dev")
}

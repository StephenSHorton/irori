//! The tray on a phone (card #226): Android and iOS have no tray, so this
//! is [`super::tray`]'s API with nothing behind it. The shell wires it the
//! same way on every platform; on mobile every call is a no-op.

use std::sync::Arc;

use tauri::AppHandle;

use super::tray_text::Counts;

pub type Action = Arc<dyn Fn() + Send + Sync>;
pub type Restart = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

pub struct Tray;

impl Tray {
    pub fn install(_app: &AppHandle, _version: &str) -> Result<Arc<Tray>, String> {
        Ok(Arc::new(Tray))
    }
    pub fn set_status(&self, _s: &str) {}
    pub fn set_inbox(&self, _c: Counts) {}
    pub fn set_decide(&self, _n: usize) {}
    pub fn set_held_opener(&self, _open: Action) {}
    pub fn set_wind_down_opener(&self, _open: Action) {}
    pub fn set_wind_down(&self, _on: bool) {}
    pub fn set_wind_down_off(&self, _off: Restart) {}
    pub fn set_update(&self, _ready: Option<(String, Restart)>) {}
    pub fn rebuild(&self) {}
}

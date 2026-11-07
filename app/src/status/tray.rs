//! The tray icon's menu (main's app/src/shell/tray.rs, Go
//! `TrayController`): rebuilt whole from [`tray_text::lines`] on every
//! change. Every method is safe from any thread; the native icon lives in
//! [`State::tray`] and its menu changes on the UI thread (macOS's
//! tray-icon must stay on the main thread).
//!
//! - Windows and Linux: Rune's `rune::tray::Tray`. It has no check items,
//!   so the two checkboxes are a "✓ " in front of the label (a Rune gap).
//!   A left click opens the window, a right click the menu.
//! - macOS: tray-icon (crate::tray_mac), with real check items; a click
//!   opens the menu, as every menu-bar item does.

use std::sync::{Arc, Mutex};

use rune::AppHandle;

use super::tray_text::{self, Counts, Line, Model, id};
use crate::ui::State;

pub type SetPause = Arc<dyn Fn(bool) -> Result<(), String> + Send + Sync>;
pub type Action = Arc<dyn Fn() + Send + Sync>;
pub type Restart = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;
/// Turns launch at login on or off (Window::set_launch_at_login, which
/// announces the live state and rebuilds this menu through the login
/// seam).
pub type SetLogin = Arc<dyn Fn(bool) -> Result<(), String> + Send + Sync>;

#[derive(Default)]
struct Inner {
    model: Model,
    set_pause: Option<SetPause>,
    open_held: Option<Action>,
    open_wind_down: Option<Action>,
    restart: Option<Restart>,
    set_login: Option<SetLogin>,
}

pub struct Tray {
    app: AppHandle<State>,
    st: Mutex<Inner>,
}

impl Tray {
    pub fn new(app: AppHandle<State>, version: &str) -> Arc<Tray> {
        Arc::new(Tray {
            app,
            st: Mutex::new(Inner {
                model: Model {
                    version: version.to_string(),
                    ..Model::default()
                },
                ..Inner::default()
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Go `SetStatus`: the status line; empty restores the version line.
    pub fn set_status(&self, s: &str) {
        self.edit(|st| {
            let changed = st.model.status != s;
            st.model.status = s.to_string();
            changed
        });
    }

    /// Go `SetInbox`: the unread and held counts and the pause state.
    pub fn set_inbox(&self, c: Counts) {
        self.edit(|st| {
            let changed = st.model.counts != c;
            st.model.counts = c;
            changed
        });
    }

    /// The pause state alone, after the tray's own toggle.
    pub fn set_paused(&self, paused: bool) {
        self.edit(|st| {
            let changed = st.model.counts.paused != paused;
            st.model.counts.paused = paused;
            changed
        });
    }

    /// Go `SetDecide`: cards waiting on a decision, every device's.
    pub fn set_decide(&self, n: usize) {
        self.edit(|st| {
            let changed = st.model.decide != n;
            st.model.decide = n;
            changed
        });
    }

    /// Go `SetPause`: shows the "Pause wakes on <device>" checkbox.
    pub fn set_pause(&self, device: &str, set: SetPause) {
        self.edit(|st| {
            st.model.device = device.to_string();
            st.model.can_pause = true;
            st.set_pause = Some(set);
            true
        });
    }

    /// Go `SetHeldOpener`: the held line opens the window's Held list.
    pub fn set_held_opener(&self, open: Action) {
        self.edit(|st| {
            st.model.can_open_held = true;
            st.open_held = Some(open);
            true
        });
    }

    /// Wind down all agents… opens the window's dialog (card #220).
    pub fn set_wind_down_opener(&self, open: Action) {
        self.edit(|st| {
            st.model.can_wind_down = true;
            st.open_wind_down = Some(open);
            true
        });
    }

    /// The launch-at-login checkbox's setter.
    pub fn set_login_setter(&self, set: SetLogin) {
        self.lock().set_login = Some(set);
    }

    /// `Some((version, restart))` while an update is ready, else None.
    pub fn set_update(&self, ready: Option<(String, Restart)>) {
        self.edit(|st| {
            let v = ready.as_ref().map(|(v, _)| v.clone());
            let changed = st.model.update_ready != v;
            st.model.update_ready = v;
            st.restart = ready.map(|(_, r)| r);
            changed
        });
    }

    fn edit(&self, f: impl FnOnce(&mut Inner) -> bool) {
        let changed = f(&mut self.lock());
        if changed {
            self.rebuild();
        }
    }

    /// The menu's lines now, with the live launch-at-login state (an OS
    /// read, so never on the UI thread).
    pub fn lines(&self) -> Vec<Line> {
        let mut m = self.lock().model.clone();
        m.login = match crate::login::LoginItem::system().is_enabled() {
            Ok(on) => on,
            Err(e) => {
                irori_core::log(&format!("irori: read launch at login: {e}"));
                false
            }
        };
        tray_text::lines(&m)
    }

    /// Rebuilds the native menu (after a launch-at-login change, say).
    pub fn rebuild(&self) {
        let lines = self.lines();
        // false: the menu isn't in the view, so the window doesn't redraw.
        self.app.update_if(move |s: &mut State, _| {
            apply(s, &lines);
            false
        });
    }

    /// Runs `f` on the UI thread (the native icon's thread on macOS)
    /// without redrawing the window.
    pub fn app_update(&self, f: impl FnOnce(&mut State) + Send + 'static) {
        self.app.update_if(move |s: &mut State, _| {
            f(s);
            false
        });
    }

    /// A menu pick (the native tray's thread).
    pub fn clicked(self: &Arc<Self>, item: &str) {
        let (paused, set_pause, open_held, open_wind_down, restart, set_login) = {
            let st = self.lock();
            (
                st.model.counts.paused,
                st.set_pause.clone(),
                st.open_held.clone(),
                st.open_wind_down.clone(),
                st.restart.clone(),
                st.set_login.clone(),
            )
        };
        match item {
            id::OPEN => self.app.open(),
            id::HELD => {
                if let Some(f) = open_held {
                    f();
                }
            }
            id::WIND_DOWN => {
                if let Some(f) = open_wind_down {
                    f();
                }
            }
            id::PAUSE => {
                let Some(set) = set_pause else { return };
                let want = !paused;
                let t = self.clone();
                std::thread::spawn(move || match set(want) {
                    Ok(()) => t.set_paused(want),
                    // Rebuild to snap the checkbox back.
                    Err(e) => {
                        irori_core::log(&format!("irori: pause all wakes: {e}"));
                        t.rebuild();
                    }
                });
            }
            id::LOGIN => {
                let t = self.clone();
                std::thread::spawn(move || {
                    let want = !crate::login::LoginItem::system()
                        .is_enabled()
                        .unwrap_or(false);
                    let r = match &set_login {
                        Some(set) => set(want),
                        None => crate::login::LoginItem::system().set(want),
                    };
                    if let Err(e) = r {
                        irori_core::log(&format!("irori: set launch at login: {e}"));
                    }
                    // Snaps the checkbox to the live state either way.
                    t.rebuild();
                });
            }
            id::UPDATE => {
                if let Some(r) = restart {
                    std::thread::spawn(move || {
                        if let Err(e) = r() {
                            irori_core::log(&format!("irori: tray restart to update: {e}"));
                        }
                    });
                }
            }
            id::QUIT => self.app.quit(),
            _ => {}
        }
    }
}

/// Puts the lines in the native menu (UI thread).
fn apply(s: &mut State, lines: &[Line]) {
    let Some(t) = s.tray.as_ref() else { return };
    #[cfg(target_os = "macos")]
    if let Some(t) = t.downcast_ref::<tray_icon::TrayIcon>() {
        crate::tray_mac::set_menu(t, lines);
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(t) = t.downcast_ref::<rune::tray::Tray>() {
        t.set_menu(items(lines));
    }
}

/// Rune's tray items for the lines: a check is a "✓ " label.
#[cfg(not(target_os = "macos"))]
pub fn items(lines: &[Line]) -> Vec<rune::tray::TrayItem> {
    use rune::tray::TrayItem;
    lines
        .iter()
        .map(|l| match l {
            Line::Item { id, label, enabled } => {
                TrayItem::action(*id, label.clone()).enabled(*enabled)
            }
            Line::Check { id, label, checked } => {
                TrayItem::action(*id, check_label(label, *checked))
            }
            Line::Separator => TrayItem::Separator,
        })
        .collect()
}

/// A check item's label where the tray has no check items.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn check_label(label: &str, checked: bool) -> String {
    // Figure space keeps the unchecked labels lined up with the checked.
    if checked {
        format!("✓ {label}")
    } else {
        format!("\u{2007}  {label}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_without_check_items() {
        assert_eq!(check_label("Launch at login", true), "✓ Launch at login");
        assert!(check_label("Launch at login", false).ends_with("  Launch at login"));
    }
}

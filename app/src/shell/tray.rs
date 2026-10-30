//! The tray icon and its menu (Go `TrayController`, I:tray.go,
//! tray_darwin.go, tray_other.go). The menu is rebuilt whole on every
//! change from [`tray_text::lines`]. Every method is safe from any thread.
//!
//! Seams for the rest of M6 (merge notes): the inbox pump calls
//! [`Tray::set_inbox`], the board pump [`Tray::set_decide`], the notifier's
//! opener goes into [`Tray::set_held_opener`], and the updater into
//! [`Tray::set_update`]. The shell itself wires the roster line, the pause
//! checkbox and launch at login.

use std::sync::{Arc, Mutex};

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Wry};

use super::tray_text::{self, Counts, Line, Model, id};

pub type SetPause = Arc<dyn Fn(bool) -> Result<(), String> + Send + Sync>;
pub type Action = Arc<dyn Fn() + Send + Sync>;
pub type Restart = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

#[derive(Default)]
struct State {
    model: Model,
    set_pause: Option<SetPause>,
    open_held: Option<Action>,
    restart: Option<Restart>,
}

pub struct Tray {
    app: AppHandle,
    icon: Mutex<Option<TrayIcon>>,
    st: Mutex<State>,
}

/// Go `trayClickOpensWindow`: on Windows a left click opens the window and
/// a right click the menu; on macOS a click opens the menu, as every menu
/// bar item does.
const CLICK_OPENS_WINDOW: bool = !cfg!(target_os = "macos");

impl Tray {
    /// Go `setupTray`: installs the icon and returns its controller.
    pub fn install(app: &AppHandle, version: &str) -> Result<Arc<Tray>, String> {
        let t = Arc::new(Tray {
            app: app.clone(),
            icon: Mutex::new(None),
            st: Mutex::new(State {
                model: Model {
                    version: version.to_string(),
                    ..Model::default()
                },
                ..State::default()
            }),
        });
        let menu = t.menu().map_err(|e| e.to_string())?;
        // Windows: the 囲 logo on a tight white tile. macOS: the mono mark
        // as a template image, recoloured for light and dark menu bars.
        let png: &[u8] = if cfg!(target_os = "macos") {
            include_bytes!("../../icons/trayicon-template.png")
        } else {
            include_bytes!("../../icons/trayicon.png")
        };
        let icon = Image::from_bytes(png).map_err(|e| e.to_string())?;
        let on_menu = Arc::downgrade(&t);
        let on_click = Arc::downgrade(&t);
        let tray = TrayIconBuilder::with_id("main")
            .icon(icon)
            .icon_as_template(cfg!(target_os = "macos"))
            .tooltip("Irori")
            .menu(&menu)
            .show_menu_on_left_click(!CLICK_OPENS_WINDOW)
            .on_menu_event(move |_app, ev| {
                if let Some(t) = on_menu.upgrade() {
                    t.clicked(ev.id().as_ref());
                }
            })
            .on_tray_icon_event(move |_tray, ev| {
                if CLICK_OPENS_WINDOW
                    && let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = ev
                    && let Some(t) = on_click.upgrade()
                {
                    super::show(&t.app);
                }
            })
            .build(app)
            .map_err(|e| e.to_string())?;
        *t.icon.lock().unwrap_or_else(|e| e.into_inner()) = Some(tray);
        Ok(t)
    }

    /// Go `SetStatus`: replaces the status line; empty restores the
    /// version line.
    pub fn set_status(&self, s: &str) {
        self.edit(|st| {
            st.model.status = s.to_string();
            true
        });
    }

    /// Go `SetInbox`: the unread and held counts and the pause state.
    /// Unchanged counts don't rebuild the menu.
    #[allow(dead_code)] // the inbox pump (M6 commands) calls it
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
    #[allow(dead_code)] // the notifier's opener (M6 toast) is wired here
    pub fn set_held_opener(&self, open: Action) {
        self.edit(|st| {
            st.model.can_open_held = true;
            st.open_held = Some(open);
            true
        });
    }

    /// Go `SetUpdater` + its `update:status` rebuilds: `Some((version,
    /// restart))` while an update is ready, `None` otherwise.
    pub fn set_update(&self, ready: Option<(String, Restart)>) {
        self.edit(|st| {
            match ready {
                Some((v, r)) => {
                    st.model.update_ready = Some(v);
                    st.restart = Some(r);
                }
                None => {
                    st.model.update_ready = None;
                    st.restart = None;
                }
            }
            true
        });
    }

    /// Rebuilds the menu (after a launch-at-login change, say).
    pub fn rebuild(&self) {
        let menu = match self.menu() {
            Ok(m) => m,
            Err(e) => {
                irori_core::log(&format!("irori: tray menu: {e}"));
                return;
            }
        };
        if let Some(icon) = self.icon.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
            && let Err(e) = icon.set_menu(Some(menu))
        {
            irori_core::log(&format!("irori: tray menu: {e}"));
        }
    }

    fn edit(&self, f: impl FnOnce(&mut State) -> bool) {
        let changed = f(&mut self.st.lock().unwrap_or_else(|e| e.into_inner()));
        if changed {
            self.rebuild();
        }
    }

    fn model(&self) -> Model {
        let mut m = self
            .st
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .model
            .clone();
        m.login = match super::login::LoginItem::system().is_enabled() {
            Ok(on) => on,
            Err(e) => {
                irori_core::log(&format!("irori: read launch at login: {e}"));
                false
            }
        };
        m
    }

    fn menu(&self) -> tauri::Result<Menu<Wry>> {
        let a = &self.app;
        let menu = Menu::new(a)?;
        for l in tray_text::lines(&self.model()) {
            match l {
                Line::Item { id, label, enabled } => {
                    menu.append(&MenuItem::with_id(a, id, label, enabled, None::<&str>)?)?
                }
                Line::Check { id, label, checked } => menu.append(&CheckMenuItem::with_id(
                    a,
                    id,
                    label,
                    true,
                    checked,
                    None::<&str>,
                )?)?,
                Line::Separator => menu.append(&PredefinedMenuItem::separator(a)?)?,
            }
        }
        Ok(menu)
    }

    fn clicked(self: &Arc<Self>, item: &str) {
        let (paused, set_pause, open_held, restart) = {
            let st = self.st.lock().unwrap_or_else(|e| e.into_inner());
            (
                st.model.counts.paused,
                st.set_pause.clone(),
                st.open_held.clone(),
                st.restart.clone(),
            )
        };
        match item {
            id::OPEN => super::show(&self.app),
            id::HELD => {
                if let Some(f) = open_held {
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
                let app = self.app.clone();
                std::thread::spawn(move || {
                    let want = !super::login::LoginItem::system()
                        .is_enabled()
                        .unwrap_or(false);
                    // Announces the live state, which rebuilds the menu
                    // (and snaps the checkbox back if it failed).
                    if let Err(e) = super::set_launch_at_login(&app, want) {
                        irori_core::log(&format!("irori: set launch at login: {e}"));
                    }
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
            id::QUIT => self.app.exit(0),
            _ => {}
        }
    }
}

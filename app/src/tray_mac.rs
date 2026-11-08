//! The macOS menu-bar item (card #674). Rune's `Tray` is Windows and Linux
//! only, so the Mac uses tray-icon (the crate Tauri's tray is built on, no
//! webview): the template icon and the menu status::tray builds, with real
//! check items. A click opens the menu, as every menu-bar item does.

use std::sync::Arc;

use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::status::tray::Tray;
use crate::status::tray_text::Line;

/// The native menu for the lines.
fn menu(lines: &[Line]) -> Result<Menu, String> {
    let menu = Menu::new();
    for l in lines {
        let item: Box<dyn IsMenuItem> = match l {
            Line::Item { id, label, enabled } => {
                Box::new(MenuItem::with_id(*id, label, *enabled, None))
            }
            Line::Check { id, label, checked } => {
                Box::new(CheckMenuItem::with_id(*id, label, true, *checked, None))
            }
            Line::Separator => Box::new(PredefinedMenuItem::separator()),
        };
        menu.append(item.as_ref()).map_err(|e| e.to_string())?;
    }
    Ok(menu)
}

/// Makes the item. On the main thread, inside the event loop (an
/// `AppHandle::update`); the returned icon must live as long as the app.
pub fn install(version: &str, lines: &[Line]) -> Result<TrayIcon, String> {
    let (w, h, px) = crate::shell::decode_png(include_bytes!("../icons/trayicon-template.png"))
        .ok_or("tray icon: bad PNG")?;
    let icon = Icon::from_rgba(px, w, h).map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_icon(icon)
        .with_icon_as_template(true)
        .with_tooltip(format!("Irori {version}"))
        .with_menu(Box::new(menu(lines)?))
        .with_menu_on_left_click(true)
        .build()
        .map_err(|e| e.to_string())
}

/// Replaces the menu (main thread).
pub fn set_menu(icon: &TrayIcon, lines: &[Line]) {
    match menu(lines) {
        Ok(m) => icon.set_menu(Some(Box::new(m))),
        Err(e) => irori_core::log(&format!("irori: tray menu: {e}")),
    }
}

/// Routes the menu's picks to the tray's controller, off the main thread.
pub fn route(tray: Arc<Tray>) {
    std::thread::spawn(move || {
        while let Ok(ev) = MenuEvent::receiver().recv() {
            tray.clicked(ev.id.0.as_str());
        }
    });
}

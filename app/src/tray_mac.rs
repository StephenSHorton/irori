//! The macOS menu-bar item (card #674). Rune's `Tray` is Windows and Linux
//! only, so the Mac uses tray-icon (the crate Tauri's tray is built on, no
//! webview): the template icon, Open Irori and Quit.

use rune::AppHandle;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::board::State;

const OPEN: &str = "open";
const QUIT: &str = "quit";

/// Makes the item. On the main thread, inside the event loop (an
/// `AppHandle::update`); the returned icon must live as long as the app.
pub fn install(version: &str) -> Result<TrayIcon, String> {
    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id(OPEN, "Open Irori", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(QUIT, "Quit Irori", true, None),
    ])
    .map_err(|e| e.to_string())?;
    let (w, h, px) = crate::shell::decode_png(include_bytes!("../icons/trayicon-template.png"))
        .ok_or("tray icon: bad PNG")?;
    let icon = Icon::from_rgba(px, w, h).map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_icon(icon)
        .with_icon_as_template(true)
        .with_tooltip(format!("Irori {version}"))
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|e| e.to_string())
}

/// Routes the item's clicks and menu picks to the app, off the main thread.
pub fn route(h: AppHandle<State>) {
    let menu = h.clone();
    std::thread::spawn(move || {
        while let Ok(ev) = MenuEvent::receiver().recv() {
            match ev.id.0.as_str() {
                OPEN => menu.open(),
                QUIT => menu.quit(),
                _ => {}
            }
        }
    });
    std::thread::spawn(move || {
        while let Ok(ev) = TrayIconEvent::receiver().recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = ev
            {
                h.open();
            }
        }
    });
}

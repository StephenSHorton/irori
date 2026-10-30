//! App-level preferences (I:settings_service.go): the notification
//! settings, the sample card, and launch at login.
//!
//! The notifier is the notify crate's ([`NotifierSeam`]): it owns
//! `settings.json`'s notifications key and its one writer (inbox.md 3.11),
//! so this crate keeps no settings.json code of its own.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Window;
use crate::seams::{LoginItem, Notifier};

pub const CORNER_TOP_RIGHT: &str = "top-right";
pub const CORNER_BOTTOM_RIGHT: &str = "bottom-right";
pub const CORNER_TOP_LEFT: &str = "top-left";
pub const CORNER_BOTTOM_LEFT: &str = "bottom-left";

/// The notification settings (Go `notify.Options`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct NotifyOptions {
    pub enabled: bool,
    pub sound: bool,
    /// 0..1.
    #[specta(type = i64)]
    pub volume: f64,
    /// One of the CORNER_* values.
    pub corner: String,
}

impl Default for NotifyOptions {
    /// On, with sound, top-right (go-toast `DefaultOptions`).
    fn default() -> Self {
        NotifyOptions {
            enabled: true,
            sound: true,
            volume: 0.6,
            corner: CORNER_TOP_RIGHT.into(),
        }
    }
}

impl NotifyOptions {
    /// Snaps hand-edited values the way go-toast does.
    pub fn normalized(mut self) -> NotifyOptions {
        if self.volume.is_nan() {
            self.volume = NotifyOptions::default().volume;
        } else {
            self.volume = self.volume.clamp(0.0, 1.0);
        }
        match self.corner.as_str() {
            CORNER_TOP_RIGHT | CORNER_BOTTOM_RIGHT | CORNER_TOP_LEFT | CORNER_BOTTOM_LEFT => {}
            _ => self.corner = CORNER_TOP_RIGHT.into(),
        }
        self
    }
}

/// Where a notification's Open goes (Go `notify.Target`); the app:open
/// payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct Target {
    /// "inbox", "board", "held".
    pub tab: String,
    #[serde(rename = "ref", default, skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub ref_: String,
}

/// The window's notifier seam on the notify crate (SettingsService's
/// notification calls): its options, its settings.json, its sample card.
pub struct NotifierSeam(pub notify::Notifier);

impl NotifierSeam {
    /// A notifier that never shows a card, over `dir`'s settings.json (the
    /// crate's default; the app passes its own).
    pub fn headless(dir: &std::path::Path) -> NotifierSeam {
        NotifierSeam(notify::Notifier::new(notify::Config {
            dir: Some(dir.to_path_buf()),
            headless: true,
            ..notify::Config::default()
        }))
    }
}

fn from_notify(o: notify::Options) -> NotifyOptions {
    NotifyOptions {
        enabled: o.enabled,
        sound: o.sound,
        volume: o.volume,
        corner: o.corner,
    }
}

impl Notifier for NotifierSeam {
    fn options(&self) -> NotifyOptions {
        from_notify(self.0.options())
    }
    fn set_options(&self, o: NotifyOptions) -> Result<NotifyOptions, String> {
        let (o, saved) = self.0.set_options(notify::Options {
            enabled: o.enabled,
            sound: o.sound,
            volume: o.volume,
            corner: o.corner,
        });
        saved?;
        Ok(from_notify(o))
    }
    fn test(&self) -> Result<(), String> {
        self.0.post_test()
    }
}

/// Launch at login this build has: none registered, and it can't
/// register one (M6 shell brings the real registration).
pub struct NoLoginItem;

impl LoginItem for NoLoginItem {
    fn enabled(&self) -> Result<bool, String> {
        Ok(false)
    }
    fn set_enabled(&self, on: bool) -> Result<(), String> {
        if on {
            return Err("launch at login isn't in this build yet".into());
        }
        Ok(())
    }
}

/// The settings:launch-at-login event.
pub const EVENT_LAUNCH_AT_LOGIN: &str = "settings:launch-at-login";

impl Window {
    /// SettingsService.GetNotifications.
    pub fn get_notifications(&self) -> NotifyOptions {
        self.seams.notifier.options()
    }

    /// SettingsService.SetNotifications: applied and saved; the options as
    /// applied.
    pub fn set_notifications(&self, o: NotifyOptions) -> Result<NotifyOptions, String> {
        self.seams.notifier.set_options(o)
    }

    /// SettingsService.TestNotification.
    pub fn test_notification(&self) -> Result<(), String> {
        self.seams.notifier.test()
    }

    /// SettingsService.GetLaunchAtLogin: the live OS state.
    pub fn get_launch_at_login(&self) -> Result<bool, String> {
        self.seams.login.enabled()
    }

    /// SettingsService.SetLaunchAtLogin: registers or removes it, and
    /// announces the live state either way, so a failed toggle snaps back.
    pub fn set_launch_at_login(&self, on: bool) -> Result<(), String> {
        let r = self.seams.login.set_enabled(on);
        if let Ok(now) = self.seams.login.enabled() {
            self.emit(EVENT_LAUNCH_AT_LOGIN, &now);
        }
        r
    }
}

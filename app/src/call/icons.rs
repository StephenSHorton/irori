//! The Lucide icons the huddle, the share stage and the mic draw, kept in
//! this package so ui/icons.rs stays as it is.

use rune::Icon;

macro_rules! lucide {
    ($name:ident, $id:literal, $body:literal) => {
        pub static $name: Icon = Icon {
            name: concat!("lucide-", $id),
            view: 24.0,
            body: concat!(
                r#"<g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"#,
                $body,
                "</g>"
            ),
        };
    };
}

pub use crate::ui::icons::{MIC, MONITOR, SEARCH, X};

lucide!(HEADPHONES, "headphones", r#"<path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"/>"#);
lucide!(HEADPHONE_OFF, "headphone-off", r#"<path d="M21 14h-1.343"/><path d="M9.128 3.47A9 9 0 0 1 21 12v3.343"/><path d="m2 2 20 20"/><path d="M20.414 20.414A2 2 0 0 1 19 21h-1a2 2 0 0 1-2-2v-3"/><path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 2.636-6.364"/>"#);
lucide!(MIC_OFF, "mic-off", r#"<path d="M12 19v3"/><path d="M15 9.34V5a3 3 0 0 0-5.68-1.33"/><path d="M16.95 16.95A7 7 0 0 1 5 12v-2"/><path d="M18.89 13.23A7 7 0 0 0 19 12v-2"/><path d="m2 2 20 20"/><path d="M9 9v3a3 3 0 0 0 5.12 2.12"/>"#);
lucide!(MONITOR_UP, "monitor-up", r#"<path d="m9 10 3-3 3 3"/><path d="M12 13V7"/><rect width="20" height="14" x="2" y="3" rx="2"/><path d="M12 17v4"/><path d="M8 21h8"/>"#);
lucide!(MONITOR_OFF, "monitor-off", r#"<path d="M12 17v4"/><path d="M17 17H4a2 2 0 0 1-2-2V5a2 2 0 0 1 1.184-1.826"/><path d="m2 2 20 20"/><path d="M8 21h8"/><path d="M8.656 3H20a2 2 0 0 1 2 2v10a2 2 0 0 1-.293 1.042"/>"#);
lucide!(APP_WINDOW, "app-window", r#"<rect x="2" y="4" width="20" height="16" rx="2"/><path d="M10 4v4"/><path d="M2 8h20"/><path d="M6 4v4"/>"#);
lucide!(PHONE_OFF, "phone-off", r#"<path d="M10.1 13.9a14 14 0 0 0 3.732 2.668 1 1 0 0 0 1.213-.303l.355-.465A2 2 0 0 1 17 15h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2 18 18 0 0 1-12.728-5.272"/><path d="M22 2 2 22"/><path d="M4.76 13.582A18 18 0 0 1 2 4a2 2 0 0 1 2-2h3a2 2 0 0 1 2 2v3a2 2 0 0 1-.8 1.6l-.468.351a1 1 0 0 0-.292 1.233 14 14 0 0 0 .244.473"/>"#);
lucide!(TRIANGLE_ALERT, "triangle-alert", r#"<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>"#);
lucide!(CIRCLE_DOT, "circle-dot", r#"<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="1"/>"#);
lucide!(FILE_TEXT, "file-text", r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"/><path d="M14 2v5a1 1 0 0 0 1 1h5"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/>"#);
lucide!(MAXIMIZE, "maximize", r#"<path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/>"#);
lucide!(MINIMIZE, "minimize", r#"<path d="M8 3v3a2 2 0 0 1-2 2H3"/><path d="M21 8h-3a2 2 0 0 1-2-2V3"/><path d="M3 16h3a2 2 0 0 1 2 2v3"/><path d="M16 21v-3a2 2 0 0 1 2-2h3"/>"#);
lucide!(LOADER, "loader-circle", r#"<path d="M21 12a9 9 0 1 1-6.219-8.56"/>"#);

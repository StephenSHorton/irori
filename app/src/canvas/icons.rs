//! The canvas's own Lucide icons (the React app's lucide-react and React
//! Flow's controls), kept here so ui/icons.rs stays untouched.

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

lucide!(MAXIMIZE_2, "maximize-2", r#"<polyline points="15 3 21 3 21 9"/><polyline points="9 21 3 21 3 15"/><line x1="21" x2="14" y1="3" y2="10"/><line x1="3" x2="10" y1="21" y2="14"/>"#);
lucide!(MINIMIZE_2, "minimize-2", r#"<polyline points="4 14 10 14 10 20"/><polyline points="20 10 14 10 14 4"/><line x1="14" x2="21" y1="10" y2="3"/><line x1="3" x2="10" y1="21" y2="14"/>"#);
lucide!(ZOOM_IN, "plus", r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#);
lucide!(ZOOM_OUT, "minus", r#"<path d="M5 12h14"/>"#);
lucide!(FIT, "scan", r#"<path d="M3 7V5a2 2 0 0 1 2-2h2"/><path d="M17 3h2a2 2 0 0 1 2 2v2"/><path d="M21 17v2a2 2 0 0 1-2 2h-2"/><path d="M7 21H5a2 2 0 0 1-2-2v-2"/>"#);

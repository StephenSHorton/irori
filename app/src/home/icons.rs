//! Home's own Lucide icons, drawn like ui/icons.rs's.

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

lucide!(
    POWER,
    "power",
    r#"<path d="M12 2v10"/><path d="M18.4 6.6a9 9 0 1 1-12.77.04"/>"#
);

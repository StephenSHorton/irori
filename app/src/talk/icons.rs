//! Talk's own Lucide icons (kept here, not in ui/icons.rs, so packages
//! don't collide there).

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

lucide!(HASH, "hash", r#"<line x1="4" x2="20" y1="9" y2="9"/><line x1="4" x2="20" y1="15" y2="15"/><line x1="10" x2="8" y1="3" y2="21"/><line x1="16" x2="14" y1="3" y2="21"/>"#);
lucide!(EYE, "eye", r#"<path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"/><circle cx="12" cy="12" r="3"/>"#);
lucide!(MESSAGE_TEXT, "message-square-text", r#"<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/><path d="M13 8H7"/><path d="M17 12H7"/>"#);
lucide!(SQUARE_PEN, "square-pen", r#"<path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z"/>"#);
lucide!(CHEVRON_DOWN, "chevron-down", r#"<path d="m6 9 6 6 6-6"/>"#);
lucide!(CHEVRON_RIGHT, "chevron-right", r#"<path d="m9 18 6-6-6-6"/>"#);
lucide!(LOG_OUT, "log-out", r#"<path d="m16 17 5-5-5-5"/><path d="M21 12H9"/><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/>"#);
lucide!(USERS, "users", r#"<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>"#);
lucide!(SQUARE_PLUS, "square-plus", r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M8 12h8"/><path d="M12 8v8"/>"#);
lucide!(ARROW_UP_RIGHT, "arrow-up-right", r#"<path d="M7 7h10v10"/><path d="M7 17 17 7"/>"#);
lucide!(PHONE_CALL, "phone", r#"<path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7A2 2 0 0 1 22 16.92z"/>"#);

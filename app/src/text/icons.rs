//! The text package's own Lucide icons (links, copy, pictures, file kinds),
//! drawn like ui/icons.rs's. Kept here so the shared list stays short.

use rune::Icon;

macro_rules! lucide {
    ($name:ident, $id:literal, $body:expr) => {
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

/// A file's page and its folded corner, the base of every file-kind icon.
macro_rules! file_icon {
    ($name:ident, $id:literal, $body:expr) => {
        lucide!(
            $name,
            $id,
            concat!(
                r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/>"#,
                $body
            )
        );
    };
}

lucide!(EXTERNAL_LINK, "external-link", r#"<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>"#);
lucide!(COPY, "copy", r#"<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"#);
lucide!(IMAGE, "image", r#"<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#);
lucide!(IMAGE_OFF, "image-off", r#"<line x1="2" x2="22" y1="2" y2="22"/><path d="M10.41 10.41a2 2 0 1 1-2.83-2.83"/><line x1="13.5" x2="6" y1="13.5" y2="21"/><line x1="18" x2="21" y1="12" y2="15"/><path d="M3.59 3.59A1.99 1.99 0 0 0 3 5v14a2 2 0 0 0 2 2h14c.55 0 1.052-.22 1.41-.59"/><path d="M21 15V5a2 2 0 0 0-2-2H9"/>"#);
lucide!(WIFI_OFF, "wifi-off", r#"<path d="M12 20h.01"/><path d="M8.5 16.429a5 5 0 0 1 7 0"/><path d="M5 12.859a10 10 0 0 1 5.17-2.69"/><path d="M19 12.859a10 10 0 0 0-2.007-1.523"/><path d="M2 8.82a15 15 0 0 1 4.177-2.643"/><path d="M22 8.82a15 15 0 0 0-11.288-3.764"/><path d="m2 2 20 20"/>"#);
lucide!(ROTATE_CW, "rotate-cw", r#"<path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/>"#);
lucide!(DOWNLOAD, "download", r#"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" x2="12" y1="15" y2="3"/>"#);
lucide!(PAPERCLIP, "paperclip", r#"<path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/>"#);
lucide!(SEND, "send", r#"<path d="M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z"/><path d="m21.854 2.147-10.94 10.939"/>"#);
lucide!(X, "x", r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#);
lucide!(PRESENTATION, "presentation", r#"<path d="M2 3h20"/><path d="M21 3v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V3"/><path d="m7 21 5-5 5 5"/>"#);
file_icon!(FILE, "file", "");
file_icon!(FILE_IMAGE, "file-image", r#"<circle cx="10" cy="12" r="2"/><path d="m20 17-1.296-1.296a2.41 2.41 0 0 0-3.408 0L9 22"/>"#);
file_icon!(FILE_VIDEO, "file-video", r#"<path d="m10 11 5 3-5 3v-6Z"/>"#);
file_icon!(FILE_AUDIO, "file-audio", r#"<path d="M8 18v-2"/><path d="M12 18v-6"/><path d="M16 18v-3"/>"#);
file_icon!(FILE_ARCHIVE, "file-archive", r#"<path d="M10 12v-1"/><path d="M10 18v-2"/><path d="M10 7V6"/><circle cx="10" cy="20" r="2"/>"#);
file_icon!(FILE_SPREADSHEET, "file-spreadsheet", r#"<path d="M8 13h2"/><path d="M14 13h2"/><path d="M8 17h2"/><path d="M14 17h2"/>"#);
file_icon!(FILE_TEXT, "file-text", r#"<path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/>"#);
file_icon!(FILE_JSON, "file-json", r#"<path d="M10 12a1 1 0 0 0-1 1v1a1 1 0 0 1-1 1 1 1 0 0 1 1 1v1a1 1 0 0 0 1 1"/><path d="M14 18a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1 1 1 0 0 1-1-1v-1a1 1 0 0 0-1-1"/>"#);
file_icon!(FILE_BRACES, "file-braces", r#"<path d="M10 12a1 1 0 0 0-1 1v1a1 1 0 0 1-1 1 1 1 0 0 1 1 1v1a1 1 0 0 0 1 1"/><path d="M14 18a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1 1 1 0 0 1-1-1v-1a1 1 0 0 0-1-1"/>"#);
file_icon!(FILE_TERMINAL, "file-terminal", r#"<path d="m8 16 2-2-2-2"/><path d="M12 18h4"/>"#);
file_icon!(FILE_KEY, "file-key", r#"<circle cx="10" cy="16" r="2"/><path d="m16 10-4.5 4.5"/><path d="m15 11 1 1"/>"#);
file_icon!(FILE_TYPE, "file-type", r#"<path d="M9 13v-1h6v1"/><path d="M12 12v6"/><path d="M11 18h2"/>"#);
file_icon!(FILE_CODE, "file-code", r#"<path d="m10 13-2 2 2 2"/><path d="m14 17 2-2-2-2"/>"#);

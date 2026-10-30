//! Go `mimeOf`: `mime.TypeByExtension` of the lowercased extension, else
//! `http.DetectContentType` of the first 512 bytes (capture.md §7.19).
//!
//! The extension table is Go 1.26's built-in one. Go on Windows also reads
//! `HKEY_CLASSES_ROOT\.<ext>\Content Type` from the registry, and on Unix
//! the system mime.types files; those are not read here, so an extension
//! only the OS knows sniffs by content instead. FileRef.mime is display
//! only (Describe), so the difference is cosmetic.

use crate::fsutil::go_ext;

/// Go 1.26 `mime.builtinTypesLower`.
const BUILTIN: &[(&str, &str)] = &[
    (".ai", "application/postscript"),
    (".apk", "application/vnd.android.package-archive"),
    (".apng", "image/apng"),
    (".avif", "image/avif"),
    (".bin", "application/octet-stream"),
    (".bmp", "image/bmp"),
    (".com", "application/octet-stream"),
    (".css", "text/css; charset=utf-8"),
    (".csv", "text/csv; charset=utf-8"),
    (".doc", "application/msword"),
    (
        ".docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    (".ehtml", "text/html; charset=utf-8"),
    (".eml", "message/rfc822"),
    (".eps", "application/postscript"),
    (".exe", "application/octet-stream"),
    (".flac", "audio/flac"),
    (".gif", "image/gif"),
    (".gz", "application/gzip"),
    (".htm", "text/html; charset=utf-8"),
    (".html", "text/html; charset=utf-8"),
    (".ico", "image/vnd.microsoft.icon"),
    (".ics", "text/calendar; charset=utf-8"),
    (".jfif", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".jpg", "image/jpeg"),
    (".js", "text/javascript; charset=utf-8"),
    (".json", "application/json"),
    (".m4a", "audio/mp4"),
    (".mjs", "text/javascript; charset=utf-8"),
    (".mp3", "audio/mpeg"),
    (".mp4", "video/mp4"),
    (".oga", "audio/ogg"),
    (".ogg", "audio/ogg"),
    (".ogv", "video/ogg"),
    (".opus", "audio/ogg"),
    (".pdf", "application/pdf"),
    (".pjp", "image/jpeg"),
    (".pjpeg", "image/jpeg"),
    (".png", "image/png"),
    (".ppt", "application/vnd.ms-powerpoint"),
    (
        ".pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    (".ps", "application/postscript"),
    (".rdf", "application/rdf+xml"),
    (".rtf", "application/rtf"),
    (".shtml", "text/html; charset=utf-8"),
    (".svg", "image/svg+xml"),
    (".text", "text/plain; charset=utf-8"),
    (".tif", "image/tiff"),
    (".tiff", "image/tiff"),
    (".txt", "text/plain; charset=utf-8"),
    (".vtt", "text/vtt; charset=utf-8"),
    (".wasm", "application/wasm"),
    (".wav", "audio/wav"),
    (".webm", "audio/webm"),
    (".webp", "image/webp"),
    (".xbl", "text/xml; charset=utf-8"),
    (".xbm", "image/x-xbitmap"),
    (".xht", "application/xhtml+xml"),
    (".xhtml", "application/xhtml+xml"),
    (".xls", "application/vnd.ms-excel"),
    (
        ".xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    (".xml", "text/xml; charset=utf-8"),
    (".xsl", "text/xml; charset=utf-8"),
    (".zip", "application/zip"),
];

/// Go `mime.TypeByExtension` over the built-in table: case-sensitive,
/// then lowercased.
pub fn type_by_extension(ext: &str) -> &'static str {
    let find = |e: &str| BUILTIN.iter().find(|(k, _)| *k == e).map(|(_, v)| *v);
    find(ext)
        .or_else(|| find(&gocompat::text::to_lower_str(ext)))
        .unwrap_or("")
}

/// Go `mimeOf(path, head)`: the type by extension, else by content.
pub fn mime_of(path: &str, head: &[u8]) -> String {
    let t = type_by_extension(&gocompat::text::to_lower_str(go_ext(path)));
    if !t.is_empty() {
        return t.to_string();
    }
    detect_content_type(head).to_string()
}

enum Sig {
    Html(&'static [u8]),
    Masked {
        mask: &'static [u8],
        pat: &'static [u8],
        skip_ws: bool,
        ct: &'static str,
    },
    Exact(&'static [u8], &'static str),
    Mp4,
    Text,
}

const fn masked(mask: &'static [u8], pat: &'static [u8], ct: &'static str) -> Sig {
    Sig::Masked {
        mask,
        pat,
        skip_ws: false,
        ct,
    }
}

/// Go `net/http.sniffSignatures`, in order.
const SIGS: &[Sig] = &[
    Sig::Html(b"<!DOCTYPE HTML"),
    Sig::Html(b"<HTML"),
    Sig::Html(b"<HEAD"),
    Sig::Html(b"<SCRIPT"),
    Sig::Html(b"<IFRAME"),
    Sig::Html(b"<H1"),
    Sig::Html(b"<DIV"),
    Sig::Html(b"<FONT"),
    Sig::Html(b"<TABLE"),
    Sig::Html(b"<A"),
    Sig::Html(b"<STYLE"),
    Sig::Html(b"<TITLE"),
    Sig::Html(b"<B"),
    Sig::Html(b"<BODY"),
    Sig::Html(b"<BR"),
    Sig::Html(b"<P"),
    Sig::Html(b"<!--"),
    Sig::Masked {
        mask: b"\xFF\xFF\xFF\xFF\xFF",
        pat: b"<?xml",
        skip_ws: true,
        ct: "text/xml; charset=utf-8",
    },
    Sig::Exact(b"%PDF-", "application/pdf"),
    Sig::Exact(b"%!PS-Adobe-", "application/postscript"),
    masked(
        b"\xFF\xFF\x00\x00",
        b"\xFE\xFF\x00\x00",
        "text/plain; charset=utf-16be",
    ),
    masked(
        b"\xFF\xFF\x00\x00",
        b"\xFF\xFE\x00\x00",
        "text/plain; charset=utf-16le",
    ),
    masked(
        b"\xFF\xFF\xFF\x00",
        b"\xEF\xBB\xBF\x00",
        "text/plain; charset=utf-8",
    ),
    Sig::Exact(b"\x00\x00\x01\x00", "image/x-icon"),
    Sig::Exact(b"\x00\x00\x02\x00", "image/x-icon"),
    Sig::Exact(b"BM", "image/bmp"),
    Sig::Exact(b"GIF87a", "image/gif"),
    Sig::Exact(b"GIF89a", "image/gif"),
    masked(
        b"\xFF\xFF\xFF\xFF\x00\x00\x00\x00\xFF\xFF\xFF\xFF\xFF\xFF",
        b"RIFF\x00\x00\x00\x00WEBPVP",
        "image/webp",
    ),
    Sig::Exact(b"\x89PNG\x0D\x0A\x1A\x0A", "image/png"),
    Sig::Exact(b"\xFF\xD8\xFF", "image/jpeg"),
    masked(
        b"\xFF\xFF\xFF\xFF\x00\x00\x00\x00\xFF\xFF\xFF\xFF",
        b"FORM\x00\x00\x00\x00AIFF",
        "audio/aiff",
    ),
    masked(b"\xFF\xFF\xFF", b"ID3", "audio/mpeg"),
    masked(b"\xFF\xFF\xFF\xFF\xFF", b"OggS\x00", "application/ogg"),
    masked(
        b"\xFF\xFF\xFF\xFF\xFF\xFF\xFF\xFF",
        b"MThd\x00\x00\x00\x06",
        "audio/midi",
    ),
    masked(
        b"\xFF\xFF\xFF\xFF\x00\x00\x00\x00\xFF\xFF\xFF\xFF",
        b"RIFF\x00\x00\x00\x00AVI ",
        "video/avi",
    ),
    masked(
        b"\xFF\xFF\xFF\xFF\x00\x00\x00\x00\xFF\xFF\xFF\xFF",
        b"RIFF\x00\x00\x00\x00WAVE",
        "audio/wave",
    ),
    Sig::Mp4,
    Sig::Exact(b"\x1A\x45\xDF\xA3", "video/webm"),
    masked(
        b"\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\xFF\xFF",
        b"\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00LP",
        "application/vnd.ms-fontobject",
    ),
    Sig::Exact(b"\x00\x01\x00\x00", "font/ttf"),
    Sig::Exact(b"OTTO", "font/otf"),
    Sig::Exact(b"ttcf", "font/collection"),
    Sig::Exact(b"wOFF", "font/woff"),
    Sig::Exact(b"wOF2", "font/woff2"),
    Sig::Exact(b"\x1F\x8B\x08", "application/x-gzip"),
    Sig::Exact(b"PK\x03\x04", "application/zip"),
    Sig::Exact(b"Rar!\x1A\x07\x00", "application/x-rar-compressed"),
    Sig::Exact(b"Rar!\x1A\x07\x01\x00", "application/x-rar-compressed"),
    Sig::Exact(b"\x00\x61\x73\x6D", "application/wasm"),
    Sig::Text,
];

fn is_ws(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

fn is_tt(b: u8) -> bool {
    b == b' ' || b == b'>'
}

impl Sig {
    fn matches(&self, data: &[u8], first_non_ws: usize) -> Option<&'static str> {
        match self {
            Sig::Exact(sig, ct) => data.starts_with(sig).then_some(*ct),
            Sig::Masked {
                mask,
                pat,
                skip_ws,
                ct,
            } => {
                let data = if *skip_ws {
                    &data[first_non_ws..]
                } else {
                    data
                };
                if pat.len() != mask.len() || data.len() < pat.len() {
                    return None;
                }
                pat.iter()
                    .enumerate()
                    .all(|(i, pb)| data[i] & mask[i] == *pb)
                    .then_some(*ct)
            }
            Sig::Html(h) => {
                let data = &data[first_non_ws..];
                if data.len() < h.len() + 1 {
                    return None;
                }
                for (i, &b) in h.iter().enumerate() {
                    let mut db = data[i];
                    if b.is_ascii_uppercase() {
                        db &= 0xDF;
                    }
                    if b != db {
                        return None;
                    }
                }
                is_tt(data[h.len()]).then_some("text/html; charset=utf-8")
            }
            Sig::Mp4 => {
                if data.len() < 12 {
                    return None;
                }
                let box_size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
                if data.len() < box_size || !box_size.is_multiple_of(4) {
                    return None;
                }
                if &data[4..8] != b"ftyp" {
                    return None;
                }
                let mut st = 8;
                while st < box_size {
                    if st != 12 && &data[st..st + 3] == b"mp4" {
                        return Some("video/mp4");
                    }
                    st += 4;
                }
                None
            }
            Sig::Text => {
                for &b in &data[first_non_ws..] {
                    if b <= 0x08
                        || b == 0x0B
                        || (0x0E..=0x1A).contains(&b)
                        || (0x1C..=0x1F).contains(&b)
                    {
                        return None;
                    }
                }
                Some("text/plain; charset=utf-8")
            }
        }
    }
}

/// Go `http.DetectContentType`: the WHATWG sniff of at most 512 bytes.
pub fn detect_content_type(data: &[u8]) -> &'static str {
    let data = &data[..data.len().min(512)];
    let first_non_ws = data.iter().position(|&b| !is_ws(b)).unwrap_or(data.len());
    for sig in SIGS {
        if let Some(ct) = sig.matches(data, first_non_ws) {
            return ct;
        }
    }
    "application/octet-stream"
}

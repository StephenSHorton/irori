//! Board attachments in the window (I:attachments_service.go,
//! I:internal/core/attachview.go; surface.md 2.11, 7.7): Info, Save and
//! Open, and the `irori` URI scheme that serves a comment's file to the
//! webview. Only Board comments' files count, never a message's.
//!
//! The scheme answers `<origin>/irori/attachment/<sha256>`, where the
//! origin is `http://irori.localhost` on Windows and `irori://localhost`
//! elsewhere (Tauri's custom-scheme URLs), so [`AttachmentInfo::url`] is
//! absolute; the page's CSP (`app/tauri.conf.json`) allows that origin in
//! `connect-src img-src media-src`.

use std::path::{Path, PathBuf};

use blob::FileRef;
use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::seams::FetchError;

/// The path prefix a comment's file loads from.
pub const ROUTE: &str = "/irori/attachment/";
/// The URI scheme the app registers.
pub const SCHEME: &str = "irori";
/// The types the route serves inline, by the file's sniffed bytes.
pub const INLINE_IMAGES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];
/// The route refuses bigger files before any fetch.
pub const MAX_PREVIEW: i64 = 32 << 20;

/// Go `core.ErrNoAttachment`.
pub const NO_ATTACHMENT: &str = "no message or card you can read carries that attachment";
const NOT_OPENABLE: &str = "Irori opens pictures, PDFs, text, documents and media only; save this one and open it yourself if you trust it";

/// The origin the scheme is served at in the webview.
pub fn origin() -> &'static str {
    if cfg!(windows) {
        "http://irori.localhost"
    } else {
        "irori://localhost"
    }
}

/// The app's own origin, the only one the scheme answers a `fetch` from
/// (the page and the scheme are different origins under Tauri).
pub fn app_origin() -> &'static str {
    if cfg!(windows) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    }
}

/// Where the window loads a comment's file.
pub fn url(sha: &str) -> String {
    format!("{}{ROUTE}{sha}", origin())
}

/// One comment file as the window shows it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct AttachmentInfo {
    pub sha256: String,
    /// The safe file name.
    pub name: String,
    pub size: i64,
    /// The attaching device's guess; the route decides by the bytes.
    pub mime: String,
    /// The name of the device it was attached on.
    pub device: String,
    /// The bytes are on this device.
    pub here: bool,
    /// Where the window loads it.
    pub url: String,
    /// Open would open it (by its name; Open checks the bytes too).
    pub openable: bool,
}

/// The types Open hands to the OS.
const OPENABLE_EXTS: [&str; 22] = [
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".heic", ".pdf", ".txt", ".md", ".log",
    ".csv", ".json", ".yaml", ".yml", ".docx", ".xlsx", ".pptx", ".mp4", ".mov", ".mp3", ".wav",
];

pub fn openable_name(name: &str) -> bool {
    let n = name.trim_end_matches(['.', ' ']);
    let ext = crate::ext(n).to_lowercase();
    OPENABLE_EXTS.contains(&ext.as_str())
}

/// A PE, ELF or Mach-O binary, or a script with a #! line.
pub fn runnable_bytes(b: &[u8]) -> bool {
    b.starts_with(b"MZ")
        || b.starts_with(b"#!")
        || b.starts_with(b"\x7fELF")
        || [
            b"\xfe\xed\xfa\xce",
            b"\xfe\xed\xfa\xcf",
            b"\xce\xfa\xed\xfe",
            b"\xcf\xfa\xed\xfe",
            b"\xca\xfe\xba\xbe",
        ]
        .iter()
        .any(|m| b == *m)
}

fn fetch_text(e: FetchError) -> String {
    match e {
        FetchError::Offline(on) => {
            format!("the file is on {on}, which isn't connected now; try again when it is")
        }
        FetchError::TimedOut => {
            "the file took too long to arrive from the device that holds it; try again".into()
        }
        FetchError::Other(s) => s,
    }
}

impl Window {
    /// Go `Core.CommentFile`.
    fn comment_file(&self, sha: &str) -> Result<FileRef, String> {
        let sha = crate::trim(sha).to_lowercase();
        if !blob::valid_id(&sha) {
            return Err("blob: id must be 64 lowercase hex characters".into());
        }
        let mut r = self
            .core
            .comment_attachment(&sha)
            .map(|r| crate::blob_ref(&r))
            .ok_or_else(|| NO_ATTACHMENT.to_string())?;
        r.sha256 = sha;
        Ok(r)
    }

    fn device_name(&self, id: &str) -> String {
        if id.is_empty() {
            return String::new();
        }
        self.core
            .certified_devices()
            .into_iter()
            .find(|d| d.cert.device_id == id && !d.cert.label.is_empty())
            .map(|d| d.cert.label)
            .unwrap_or_else(|| id.to_string())
    }

    fn info(&self, r: &FileRef) -> AttachmentInfo {
        let name = blob::safe_name(&r.name, &r.sha256);
        AttachmentInfo {
            sha256: r.sha256.clone(),
            openable: openable_name(&name),
            name,
            size: r.size,
            mime: r.mime.clone(),
            device: self.device_name(&r.device),
            here: self.core.files().has(&r.sha256),
            url: url(&r.sha256),
        }
    }

    /// AttachmentsService.Info: a comment's file, without fetching it.
    pub fn attachment_info(&self, sha: &str) -> Result<AttachmentInfo, String> {
        let r = self.comment_file(sha)?;
        Ok(self.info(&r))
    }

    /// AttachmentsService.Fetch: brings the file here and describes it.
    pub fn attachment_fetch(&self, sha: &str) -> Result<AttachmentInfo, String> {
        let r = self.comment_file(sha)?;
        self.seams.mesh.fetch_file(&r).map_err(fetch_text)?;
        Ok(self.info(&r))
    }

    /// AttachmentsService.Save: into Downloads under its safe name, never
    /// overwriting; returns the path.
    pub fn attachment_save(&self, sha: &str) -> Result<String, String> {
        let dir = (self.cfg.downloads)()?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let r = self.comment_file(sha)?;
        self.save_ref(&r, &dir)
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// AttachmentsService.Open: with the OS default app, from a copy in a
    /// temporary folder. Only openable names and never runnable bytes.
    pub fn attachment_open(&self, sha: &str) -> Result<(), String> {
        let Some(open) = self.open_file.get() else {
            return Err("this Irori has no window to open files from".into());
        };
        let r = self.comment_file(sha)?;
        if !openable_name(&blob::safe_name(&r.name, &r.sha256)) {
            return Err(NOT_OPENABLE.into());
        }
        self.seams.mesh.fetch_file(&r).map_err(fetch_text)?;
        let mut head = [0u8; 4];
        let n = {
            use std::io::Read as _;
            let mut f = self
                .core
                .files()
                .open_file(&r.sha256)
                .map_err(|e| e.to_string())?;
            f.read(&mut head).unwrap_or(0)
        };
        if runnable_bytes(&head[..n]) {
            return Err(NOT_OPENABLE.into());
        }
        let dir = std::env::temp_dir()
            .join("irori-attachments")
            .join(&r.sha256[..16]);
        let path = self.save_ref(&r, &dir)?;
        let base = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !openable_name(&base) {
            return Err(NOT_OPENABLE.into());
        }
        open(&path)
    }

    /// Go `Core.SaveFileRef`: `dir` must be absolute and outside the data
    /// dir; the copy is marked as from elsewhere.
    fn save_ref(&self, r: &FileRef, dir: &Path) -> Result<PathBuf, String> {
        if !dir.is_absolute() {
            return Err(format!(
                "{} is not an absolute folder",
                gocompat::quote::quote(&dir.to_string_lossy())
            ));
        }
        let data = std::path::absolute(self.core.data_dir()).map_err(|e| e.to_string())?;
        let real_data = std::fs::canonicalize(&data).unwrap_or(data.clone());
        let d = gocompat::filepath::clean(&dir.to_string_lossy());
        if blob::within(&d, &data.to_string_lossy())
            || blob::within(
                &resolve_existing(Path::new(&d)).to_string_lossy(),
                &simplify(&real_data),
            )
        {
            return Err(format!("{d} is inside Irori's data dir"));
        }
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        self.seams.mesh.fetch_file(r).map_err(fetch_text)?;
        let mut dst = free_name(
            &Path::new(&d).join(blob::safe_name(&r.name, &r.sha256)),
            &r.sha256,
        );
        if blob::same_content(&dst, &r.sha256) {
            return Ok(dst);
        }
        if let Err(e) = self.core.files().copy_out(&r.sha256, &dst) {
            if !dst.exists() {
                return Err(e.to_string());
            }
            dst = free_name(&dst, &r.sha256);
            self.core
                .files()
                .copy_out(&r.sha256, &dst)
                .map_err(|e| e.to_string())?;
        }
        if let Err(e) = blob::mark_downloaded(&dst) {
            irori_core::log(&format!(
                "attachments: mark {} as from elsewhere: {e}",
                dst.display()
            ));
        }
        Ok(dst)
    }

    /// Serves one request on the `irori` scheme (Go `AttachmentHandler`).
    pub fn serve_attachment(&self, req: &Request) -> Response {
        let mut resp = Response::new(200);
        resp.header("X-Content-Type-Options", "nosniff");
        resp.header("Content-Security-Policy", "default-src 'none'; sandbox");
        if req.method != "GET" {
            resp.header("Allow", "GET");
            return resp.text(405, "method not allowed");
        }
        let Some(sha) = req.path.strip_prefix(ROUTE).filter(|s| !s.contains('/')) else {
            return resp.text(404, "404 page not found");
        };
        let r = match self.comment_file(sha) {
            Ok(r) => r,
            Err(_) => return resp.no_store().text(404, NO_ATTACHMENT),
        };
        if r.size > MAX_PREVIEW {
            return resp.no_store().text(
                413,
                &format!(
                    "the file is too big to preview here (over {} MB); save it or open it instead",
                    MAX_PREVIEW >> 20
                ),
            );
        }
        match self.seams.mesh.fetch_file(&r) {
            Ok(()) => {}
            Err(e @ FetchError::Offline(_)) => return resp.no_store().text(503, &fetch_text(e)),
            Err(e @ FetchError::TimedOut) => return resp.no_store().text(504, &fetch_text(e)),
            Err(e) => return resp.no_store().text(502, &fetch_text(e)),
        }
        let body = match std::fs::read(self.core.files().path(&r.sha256)) {
            Ok(b) => b,
            Err(e) => return resp.no_store().text(502, &e.to_string()),
        };
        let sniffed = blob::detect_content_type(&body[..body.len().min(512)]);
        let typ = sniffed.split(';').next().unwrap_or("").trim();
        let name = blob::safe_name(&r.name, &r.sha256);
        let (typ, disp) = if INLINE_IMAGES.contains(&typ) {
            (typ, "inline")
        } else {
            ("application/octet-stream", "attachment")
        };
        resp.header("Content-Type", typ);
        resp.header("Content-Disposition", &format_disposition(disp, &name));
        resp.header("Cache-Control", "private, max-age=31536000, immutable");
        let etag = format!("\"{}\"", r.sha256);
        resp.header("ETag", &etag);
        resp.header("Accept-Ranges", "bytes");
        serve_content(resp, req, &etag, body)
    }
}

fn simplify(p: &Path) -> String {
    let s = p.to_string_lossy();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

/// The longest existing prefix of `p` with symlinks resolved, the rest
/// appended (Go `resolveExisting`).
fn resolve_existing(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cur = p.to_path_buf();
    loop {
        if let Ok(real) = std::fs::canonicalize(&cur) {
            let mut out = PathBuf::from(simplify(&real));
            for r in rest.iter().rev() {
                out.push(r);
            }
            return out;
        }
        match (cur.parent(), cur.file_name()) {
            (Some(up), Some(name)) => {
                rest.push(name.to_os_string());
                cur = up.to_path_buf();
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// `dst`, or with " (2)", " (3)" ... before its extension when another
/// file holds that name (one with these bytes is the answer).
fn free_name(dst: &Path, sha: &str) -> PathBuf {
    if std::fs::symlink_metadata(dst).is_err() || blob::same_content(dst, sha) {
        return dst.to_path_buf();
    }
    let s = dst.to_string_lossy().into_owned();
    let ext = crate::ext(&s);
    let base = &s[..s.len() - ext.len()];
    let mut i = 2;
    loop {
        let p = PathBuf::from(format!("{base} ({i}){ext}"));
        if std::fs::symlink_metadata(&p).is_err() || blob::same_content(&p, sha) {
            return p;
        }
        i += 1;
    }
}

/// Go `mime.FormatMediaType(disp, {filename: name})`: a token name as is,
/// a name with only printable ASCII quoted, else RFC 2231 `filename*`.
pub fn format_disposition(disp: &str, name: &str) -> String {
    const TSPECIALS: &str = "()<>@,;:\\\"/[]?=";
    let is_token = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii() && c > ' ' && c != '\x7f' && !TSPECIALS.contains(c));
    if is_token {
        return format!("{disp}; filename={name}");
    }
    if name
        .chars()
        .all(|c| c.is_ascii() && (c >= ' ' || c == '\t') && c != '\x7f')
    {
        let mut q = String::with_capacity(name.len() + 2);
        q.push('"');
        for c in name.chars() {
            if c == '"' || c == '\\' {
                q.push('\\');
            }
            q.push(c);
        }
        q.push('"');
        return format!("{disp}; filename={q}");
    }
    let mut enc = String::new();
    for b in name.bytes() {
        let c = b as char;
        if b.is_ascii()
            && c > ' '
            && c != '\x7f'
            && !TSPECIALS.contains(c)
            && c != '*'
            && c != '\''
            && c != '%'
        {
            enc.push(c);
        } else {
            enc.push_str(&format!("%{b:02X}"));
        }
    }
    format!("{disp}; filename*=utf-8''{enc}")
}

/// One scheme request, as the app hands it over.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    /// The URL path, not decoded.
    pub path: String,
    /// The `Range` header, "" for none.
    pub range: String,
    /// The `If-None-Match` header, "" for none.
    pub if_none_match: String,
}

/// One scheme answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    fn new(status: u16) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    fn header(&mut self, k: &str, v: &str) {
        self.headers.retain(|(h, _)| !h.eq_ignore_ascii_case(k));
        self.headers.push((k.into(), v.into()));
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(h, _)| h.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    }

    fn no_store(mut self) -> Response {
        self.header("Cache-Control", "no-store");
        self
    }

    /// Go `http.Error`: plain text and a newline.
    fn text(mut self, status: u16, msg: &str) -> Response {
        self.status = status;
        self.header("Content-Type", "text/plain; charset=utf-8");
        self.body = format!("{msg}\n").into_bytes();
        self
    }
}

/// The parts of Go `http.ServeContent` the window uses: If-None-Match and
/// one byte range.
fn serve_content(mut resp: Response, req: &Request, etag: &str, body: Vec<u8>) -> Response {
    let inm = req.if_none_match.trim();
    if !inm.is_empty()
        && (inm == "*"
            || inm
                .split(',')
                .any(|t| t.trim().trim_start_matches("W/") == etag))
    {
        resp.status = 304;
        resp.headers.retain(|(h, _)| {
            !h.eq_ignore_ascii_case("Content-Type") && !h.eq_ignore_ascii_case("Content-Length")
        });
        return resp;
    }
    let size = body.len() as u64;
    let range = req.range.trim();
    if let Some(spec) = range.strip_prefix("bytes=") {
        let parsed = (|| -> Option<(u64, u64)> {
            if spec.contains(',') {
                return None;
            }
            let (a, b) = spec.split_once('-')?;
            let (a, b) = (a.trim(), b.trim());
            if a.is_empty() {
                let n: u64 = b.parse().ok()?;
                if n == 0 || size == 0 {
                    return None;
                }
                return Some((size.saturating_sub(n), size - 1));
            }
            let start: u64 = a.parse().ok()?;
            if start >= size {
                return None;
            }
            let end = if b.is_empty() {
                size - 1
            } else {
                b.parse::<u64>().ok()?.min(size - 1)
            };
            (end >= start).then_some((start, end))
        })();
        match parsed {
            Some((s, e)) => {
                resp.status = 206;
                resp.header("Content-Range", &format!("bytes {s}-{e}/{size}"));
                resp.body = body[s as usize..=e as usize].to_vec();
            }
            None => {
                resp.status = 416;
                resp.header("Content-Range", &format!("bytes */{size}"));
                resp.header("Content-Type", "text/plain; charset=utf-8");
                resp.body = b"invalid range: failed to overlap\n".to_vec();
            }
        }
        return resp;
    }
    resp.body = body;
    resp
}

/// The person's Downloads folder: the known folder, else Downloads in
/// their home.
pub fn downloads_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    if let Some(p) = known_downloads() {
        return Ok(p);
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|v| !v.is_empty())
        .ok_or("paths: no home directory")?;
    Ok(PathBuf::from(home).join("Downloads"))
}

#[cfg(windows)]
fn known_downloads() -> Option<PathBuf> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Downloads, SHGetKnownFolderPath};
    let mut p: windows_sys::core::PWSTR = std::ptr::null_mut();
    // SAFETY: p receives a CoTaskMem string we free below; the GUID is a
    // static.
    let hr = unsafe { SHGetKnownFolderPath(&FOLDERID_Downloads, 0, std::ptr::null_mut(), &mut p) };
    if hr != 0 || p.is_null() {
        if !p.is_null() {
            // SAFETY: allocated by the shell.
            unsafe { CoTaskMemFree(p as _) };
        }
        return None;
    }
    // SAFETY: a NUL-terminated wide string from the shell.
    let len = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
    // SAFETY: len wide chars are readable.
    let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) });
    // SAFETY: allocated by the shell.
    unsafe { CoTaskMemFree(p as _) };
    (!s.is_empty()).then(|| PathBuf::from(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openable() {
        assert!(openable_name("a.PNG"));
        assert!(openable_name("notes.md. "));
        assert!(!openable_name("run.exe"));
        assert!(!openable_name("page.html"));
        assert!(runnable_bytes(b"MZ\x90\x00"));
        assert!(runnable_bytes(b"\xcf\xfa\xed\xfe"));
        assert!(!runnable_bytes(b"\x89PNG"));
    }

    #[test]
    fn dispositions() {
        assert_eq!(
            format_disposition("inline", "a.png"),
            "inline; filename=a.png"
        );
        assert_eq!(
            format_disposition("attachment", "my file.txt"),
            "attachment; filename=\"my file.txt\""
        );
        assert_eq!(
            format_disposition("attachment", "é.txt"),
            "attachment; filename*=utf-8''%C3%A9.txt"
        );
    }

    #[test]
    fn ranges_and_etags() {
        let body = b"0123456789".to_vec();
        let base = Response::new(200);
        let r = serve_content(
            base.clone(),
            &Request {
                range: "bytes=2-4".into(),
                ..Default::default()
            },
            "\"x\"",
            body.clone(),
        );
        assert_eq!((r.status, r.body.as_slice()), (206, &b"234"[..]));
        assert_eq!(r.get("Content-Range"), Some("bytes 2-4/10"));
        let r = serve_content(
            base.clone(),
            &Request {
                range: "bytes=-3".into(),
                ..Default::default()
            },
            "\"x\"",
            body.clone(),
        );
        assert_eq!(r.body, b"789");
        let r = serve_content(
            base.clone(),
            &Request {
                range: "bytes=20-".into(),
                ..Default::default()
            },
            "\"x\"",
            body.clone(),
        );
        assert_eq!(r.status, 416);
        let r = serve_content(
            base.clone(),
            &Request {
                if_none_match: "\"x\"".into(),
                ..Default::default()
            },
            "\"x\"",
            body.clone(),
        );
        assert_eq!((r.status, r.body.len()), (304, 0));
        let r = serve_content(base, &Request::default(), "\"x\"", body.clone());
        assert_eq!((r.status, r.body), (200, body));
    }
}

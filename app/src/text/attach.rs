//! Files on comments, posts and messages, and files waiting to go (the
//! React app's components/attachments/*, lib/attachments.ts, uploads.ts,
//! clipboard.ts): pictures as thumbnails that open full size (a big one
//! loads on a click), everything else as a chip with Save, and Open for
//! types the OS shows rather than runs. A file on another device is fetched
//! when it shows; when that device is offline it says so and offers Retry.
//!
//! Pictures load as bytes from the backend (`Window::serve_attachment`, the
//! route the webview used) off the UI thread and decode with Rune's
//! `ImageCache`; nothing loads from a URL.

use std::sync::{Arc, LazyLock};

use base64::Engine as _;
use irori_window::attachments::{INLINE_IMAGES, MAX_PREVIEW, ROUTE, Request};
use irori_window::board::UploadFile;
use rune::{El, Icon, Image, ImageCache, SurfaceFit, Tx, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Dialog, IconButton, IconButtonVariant, Spinner,
};
use serde::Serialize;

use super::icons;
use super::rich::plain_line;
use crate::ui::{Frame, State};

/// A file as the window shows it: any of the backend's file refs
/// (`ts::FileRef`, `coord::FileRef`) through [`File::of`].
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct File {
    pub sha256: String,
    pub name: String,
    pub size: i64,
    pub mime: String,
}

impl File {
    /// Any of the backend's file refs, by its fields.
    pub fn of<T: Serialize>(r: &T) -> File {
        serde_json::to_value(r)
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    /// Each of `rs` as a [`File`].
    pub fn all<T: Serialize>(rs: &[T]) -> Vec<File> {
        rs.iter().map(File::of).collect()
    }

    /// The name to show: one plain line.
    pub fn shown(&self) -> String {
        let n = plain_line(&self.name);
        if n.is_empty() { "attachment".into() } else { n }
    }
}

/// A picture up to this size loads by itself when it scrolls near; a
/// bigger one waits for a click (it crosses devices whole).
pub const MAX_AUTO_PREVIEW: i64 = 8 << 20;
/// board_service.go's MaxUpload and inbox.MaxAttachments.
pub const MAX_UPLOAD: usize = 32 << 20;
pub const MAX_UPLOAD_FILES: usize = 10;

const IMAGE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

fn ext(name: &str) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[dot + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// A file should get a thumbnail: by the attaching device's type, else
/// its name, and no bigger than MAX_PREVIEW. The route has the last word.
pub fn looks_like_image(f: &File) -> bool {
    if f.size > MAX_PREVIEW {
        return false;
    }
    let m = f
        .mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    INLINE_IMAGES.contains(&m.as_str())
        || (m.is_empty() && IMAGE_EXTS.contains(&ext(&f.name).as_str()))
}

/// Open would take a file, by its name (the backend checks the bytes too).
pub fn can_open(f: &File) -> bool {
    irori_window::attachments::openable_name(f.name.trim_end_matches(['.', ' ']))
}

/// blob.humanSize.
pub fn human_size(n: i64) -> String {
    let f = n as f64;
    if n < 1024 {
        format!("{n} B")
    } else if n < 1 << 20 {
        format!("{:.1} KB", f / 1024.0)
    } else if n < 1 << 30 {
        format!("{:.1} MB", f / f64::from(1 << 20))
    } else {
        format!("{:.1} GB", f / f64::from(1 << 30))
    }
}

/// The icon for a file's kind (card #73): by its extension, else its mime
/// type, else a plain page.
pub fn file_kind_icon(name: &str, mime: &str) -> &'static Icon {
    let by_ext: &[(&str, &'static Icon)] = &[
        (
            "png jpg jpeg gif webp bmp heic heif tif tiff svg avif ico psd",
            &icons::FILE_IMAGE,
        ),
        ("mp4 mov m4v webm mkv avi", &icons::FILE_VIDEO),
        ("mp3 wav m4a aac flac ogg opus aiff", &icons::FILE_AUDIO),
        ("zip tar gz tgz bz2 xz 7z rar zst dmg", &icons::FILE_ARCHIVE),
        ("csv tsv xls xlsx numbers ods", &icons::FILE_SPREADSHEET),
        ("ppt pptx key odp", &icons::PRESENTATION),
        (
            "txt md markdown rtf doc docx pages odt pdf log",
            &icons::FILE_TEXT,
        ),
        ("json jsonl ndjson", &icons::FILE_JSON),
        ("yaml yml toml xml ini conf cfg plist", &icons::FILE_BRACES),
        ("sh bash zsh fish ps1 bat cmd", &icons::FILE_TERMINAL),
        ("pem key crt cer p12", &icons::FILE_KEY),
        ("ttf otf woff woff2", &icons::FILE_TYPE),
        (
            "js jsx ts tsx mjs cjs rs go py rb java kt swift c h cc cpp hpp cs php lua sql html css scss vue svelte",
            &icons::FILE_CODE,
        ),
    ];
    let e = ext(name);
    if !e.is_empty()
        && let Some((_, i)) = by_ext
            .iter()
            .find(|(exts, _)| exts.split(' ').any(|x| x == e))
    {
        return i;
    }
    let m = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if m.starts_with("image/") {
        &icons::FILE_IMAGE
    } else if m.starts_with("video/") {
        &icons::FILE_VIDEO
    } else if m.starts_with("audio/") {
        &icons::FILE_AUDIO
    } else if m.starts_with("text/") || m == "application/pdf" {
        &icons::FILE_TEXT
    } else if m == "application/json" {
        &icons::FILE_JSON
    } else if m == "application/zip" || m.contains("compressed") {
        &icons::FILE_ARCHIVE
    } else {
        &icons::FILE
    }
}

/// What loading a file came to.
#[derive(Clone)]
pub enum Loaded {
    Loading,
    Image(Image),
    /// The route serves it as a download: a chip.
    File,
    Error {
        message: String,
        offline: bool,
    },
}

/// Pictures decode on Rune's workers; a load waits for its decode off the
/// UI thread, so it needs no wake.
static IMAGES: LazyLock<ImageCache> = LazyLock::new(|| ImageCache::new(Arc::new(|| {})));

/// Decodes PNG, JPEG (GIF and WebP don't decode natively: an error).
pub fn decode(bytes: Vec<u8>) -> Result<Image, String> {
    let img = IMAGES.load(bytes);
    img.wait().map_err(|e| e.0.clone())?;
    Ok(img)
}

/// Fetches one file through the backend's route (lib/attachments.ts
/// `load`): an image decoded, a download, or why not (503: the devices
/// holding it are offline).
fn fetch(w: &irori_window::Window, sha: &str) -> Loaded {
    let resp = w.serve_attachment(&Request {
        method: "GET".into(),
        path: format!("{ROUTE}{sha}"),
        ..Request::default()
    });
    if resp.status != 200 && resp.status != 206 {
        let msg = String::from_utf8_lossy(&resp.body).trim().to_string();
        return Loaded::Error {
            message: if msg.is_empty() {
                format!("couldn't load the file ({})", resp.status)
            } else {
                msg
            },
            offline: resp.status == 503,
        };
    }
    let typ = resp
        .get("Content-Type")
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if !INLINE_IMAGES.contains(&typ) {
        return Loaded::File;
    }
    match decode(resp.body) {
        Ok(img) => Loaded::Image(img),
        Err(e) => Loaded::Error {
            message: format!("can't show this picture here: {e}"),
            offline: false,
        },
    }
}

/// Loads a file (off the UI thread) unless it is loaded or loading.
pub fn load(s: &mut State, sha: &str, again: bool) {
    if !again && s.text.files.contains_key(sha) {
        return;
    }
    let sha = sha.to_string();
    s.text.files.insert(sha.clone(), Loaded::Loading);
    let id = sha.clone();
    s.spawn(
        move |w| fetch(w, &id),
        move |s, l| {
            s.text.files.insert(sha, l);
        },
    );
}

/// The route's offline reason for a thumbnail: the device's name when it
/// can find it.
fn offline_line(message: &str) -> String {
    let rest = message.strip_prefix("the file is on ");
    match rest.and_then(|r| r.find(", which isn't connected").map(|i| &r[..i])) {
        Some(dev) => format!("On {dev}, offline"),
        None => "Its device is offline".into(),
    }
}

/// The files on a comment or post: pictures as thumbnails, the rest as
/// chips. `compact` lists every file as a chip (pictures shown elsewhere).
pub fn attachment_list(s: &State, f: &mut Frame, key: &str, files: &[File], compact: bool) -> El {
    let mut out = col().key(key.to_string()).gap(8.0).min_w(0.0);
    if files.is_empty() {
        return out;
    }
    let (images, chips): (Vec<&File>, Vec<&File>) = if compact {
        (Vec::new(), files.iter().collect())
    } else {
        files.iter().partition(|x| looks_like_image(x))
    };
    if !images.is_empty() {
        let mut r = row().key(format!("{key}/images")).gap(8.0).flex_wrap();
        for x in images {
            r = r.child(thumbnail(s, f, &format!("{key}/img-{}", x.sha256), x));
        }
        out = out.child(r);
    }
    if !chips.is_empty() {
        let mut r = row().key(format!("{key}/files")).gap(8.0).flex_wrap();
        for x in chips {
            r = r.child(file_chip(s, f, &format!("{key}/file-{}", x.sha256), x));
        }
        out = out.child(r);
    }
    out
}

fn thumbnail(s: &State, f: &mut Frame, key: &str, file: &File) -> El {
    let k = f.k;
    let name = file.shown();
    let state = s.text.files.get(&file.sha256);
    if matches!(state, Some(Loaded::File)) {
        return file_chip(s, f, key, file);
    }
    let frame = || {
        col()
            .w(160.0)
            .h(112.0)
            .radius(8.0)
            .border(1.0, k.c.border_base)
            .center()
            .gap(6.0)
            .padding(8.0)
            .clip_rounded(8.0)
    };
    let sha = file.sha256.clone();
    let wanted = file.size <= MAX_AUTO_PREVIEW || s.text.wanted.contains(&file.sha256);
    let body = match state {
        Some(Loaded::Image(img)) => {
            let full = file.clone();
            el().w(160.0)
                .h(112.0)
                .radius(8.0)
                .border(1.0, k.c.border_base)
                .fill(k.c.layer_02)
                .clip_rounded(8.0)
                .pixels_fit(img.pixels(), SurfaceFit::Fill)
                .cursor(rune::Cursor::Pointer)
                .role("button")
                .label(format!("{name} ({})", human_size(file.size)))
                .help(format!("{name} ({})", human_size(file.size)))
                .on_press(move |s: &mut State| s.text.full = Some(full.clone()))
        }
        Some(Loaded::Error { message, offline }) => {
            let again = sha.clone();
            frame()
                .fill(k.c.layer_01)
                .help(message.clone())
                .child(
                    icon(
                        if *offline {
                            &icons::WIFI_OFF
                        } else {
                            &icons::IMAGE_OFF
                        },
                        16.0,
                    )
                    .color(k.c.icon_muted),
                )
                .child(
                    text(if *offline {
                        offline_line(message)
                    } else {
                        "Couldn't load it".into()
                    })
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .max_lines(2),
                )
                .child(
                    Button::new("Retry")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .icon(&icons::ROTATE_CW)
                        .on_press(move |s: &mut State| load(s, &again, true))
                        .build(k),
                )
        }
        _ if !wanted => {
            let want = sha.clone();
            frame()
                .fill(k.c.layer_01)
                .child(icon(&icons::IMAGE, 16.0).color(k.c.icon_muted))
                .child(
                    Button::new(format!("Load preview ({})", human_size(file.size)))
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut State| {
                            s.text.wanted.insert(want.clone());
                            load(s, &want, false);
                        })
                        .build(k),
                )
        }
        _ => frame()
            .fill(k.c.layer_02)
            .label(format!("Loading {name}"))
            .child(Spinner::new().size(16.0).build(k)),
    };
    let load_sha = sha.clone();
    col()
        .key(key.to_string())
        .w(160.0)
        .gap(4.0)
        .child(body)
        .child(
            text(name.clone())
                .font(k.ty.small)
                .color(k.c.text_muted)
                .truncate()
                .help(name),
        )
        .on_visible(move |s: &mut State, on, _| {
            if on && wanted {
                load(s, &load_sha, false);
            }
        })
}

fn file_chip(s: &State, f: &mut Frame, key: &str, file: &File) -> El {
    let k = f.k;
    let name = file.shown();
    row()
        .key(key.to_string())
        .min_w(0.0)
        .gap(8.0)
        .center_y()
        .radius(8.0)
        .border(1.0, k.c.border_base)
        .fill(k.c.layer_01)
        .py(4.0)
        .pl(8.0)
        .pr(4.0)
        .child(icon(file_kind_icon(&file.name, &file.mime), 16.0).color(k.c.icon_muted))
        .child(
            text(name.clone())
                .font(k.ty.ui)
                .color(k.c.text_base)
                .truncate()
                .shrink()
                .help(name),
        )
        .child(
            text(human_size(file.size))
                .font(k.ty.small)
                .color(k.c.text_muted),
        )
        .child(file_actions(s, f, &format!("{key}/acts"), file, true))
}

/// Save (to Downloads) and, for a type the OS shows rather than runs, Open
/// (with the OS default app), with what happened on one line.
pub fn file_actions(s: &State, f: &mut Frame, key: &str, file: &File, compact: bool) -> El {
    let k = f.k;
    let busy = s.text.busy.get(&file.sha256).copied();
    let size = if compact {
        ControlSize::Small
    } else {
        ControlSize::Regular
    };
    let save = file.sha256.clone();
    let mut r = row()
        .key(key.to_string())
        .gap(4.0)
        .center_y()
        .min_w(0.0)
        .child(
            Button::new("Save")
                .variant(ButtonVariant::Ghost)
                .control_size(size)
                .icon(&icons::DOWNLOAD)
                .disabled(busy.is_some())
                .help("Save to Downloads")
                .on_press(move |s: &mut State| run(s, &save, "save"))
                .build(k),
        );
    if can_open(file) {
        let open = file.sha256.clone();
        r = r.child(
            Button::new("Open")
                .variant(ButtonVariant::Ghost)
                .control_size(size)
                .icon(&icons::EXTERNAL_LINK)
                .disabled(busy.is_some())
                .help("Open with the default app")
                .on_press(move |s: &mut State| run(s, &open, "open"))
                .build(k),
        );
    }
    if busy.is_some() {
        r = r.child(Spinner::new().size(12.0).build(k));
    }
    if let Some((note, error)) = s.text.notes.get(&file.sha256) {
        let mut line = row().gap(4.0).center_y().min_w(0.0).max_w(256.0);
        if *error && note.contains("isn't connected") {
            line = line.child(icon(&icons::WIFI_OFF, 12.0).color(k.c.danger.fg));
        }
        r = r.child(
            line.child(
                text(note.clone())
                    .font(k.ty.small)
                    .color(if *error {
                        k.c.danger.fg
                    } else {
                        k.c.text_muted
                    })
                    .truncate()
                    .shrink()
                    .help(note.clone()),
            ),
        );
    }
    r
}

/// Save or Open one file, off the UI thread.
fn run(s: &mut State, sha: &str, what: &'static str) {
    let id = sha.to_string();
    s.text.busy.insert(id.clone(), what);
    s.text.notes.remove(&id);
    let key = id.clone();
    s.spawn(
        move |w| match what {
            "save" => w.attachment_save(&id).map(|p| format!("Saved to {p}")),
            _ => w.attachment_open(&id).map(|()| String::new()),
        },
        move |s, r| {
            s.text.busy.remove(&key);
            match r {
                Ok(note) if note.is_empty() => {}
                Ok(note) => {
                    s.text.notes.insert(key, (note, false));
                }
                Err(e) => {
                    s.text.notes.insert(key, (e, true));
                }
            }
        },
    );
}

/// The full-size picture a thumbnail opened, over the page.
pub fn full_view(s: &State, f: &mut Frame) {
    let Some(file) = &s.text.full else { return };
    let Some(Loaded::Image(img)) = s.text.files.get(&file.sha256) else {
        return;
    };
    let k = f.k;
    let (w, h) = img
        .size()
        .map_or((800.0, 600.0), |[w, h]| (w as f32, h as f32));
    let win =
        f.cx.window_layout("main")
            .map_or((1100.0, 720.0), |r| (r[2], r[3]));
    // React: at most 90vw wide, the picture at most 75vh tall; the title
    // and the Save/Open row must stay on screen too.
    let scale = ((win.0 * 0.9 - 48.0) / w)
        .min((win.1 * 0.75 - 140.0).max(120.0) / h)
        .min(1.0);
    let actions = file_actions(s, f, "text-full/acts", file, false);
    Dialog::new("text-full", file.shown())
        .open(true)
        .fit()
        .description(human_size(file.size))
        .width((w * scale + 48.0).clamp(360.0, 1200.0))
        .click_outside(true)
        .on_close(|s: &mut State| s.text.full = None)
        .child(
            row()
                .key("text-full/pic")
                .w_full()
                .justify_center()
                .radius(8.0)
                .fill(k.c.layer_01)
                .child(
                    el().w(w * scale)
                        .h(h * scale)
                        .pixels_fit(img.pixels(), SurfaceFit::Fit),
                ),
        )
        .child(actions)
        .build(k, f.layer);
}

/// One file waiting to go: its name and bytes, and a preview when it is a
/// picture this window can draw.
#[derive(Clone)]
pub struct Pending {
    pub name: String,
    pub mime: String,
    pub bytes: Arc<[u8]>,
    pub preview: Option<Image>,
    /// Keeps its row (and preview) when another before it goes.
    pub id: u64,
}

impl Pending {
    /// As the backend takes it: its name and base64 bytes.
    pub fn upload(&self) -> UploadFile {
        UploadFile {
            name: if self.name.is_empty() {
                "pasted.png".into()
            } else {
                self.name.clone()
            },
            data: base64::engine::general_purpose::STANDARD.encode(&self.bytes),
        }
    }
}

/// A file's type by its name, for a picked or dropped file.
pub fn mime_of(name: &str) -> String {
    match ext(name).as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "txt" | "log" | "md" => "text/plain",
        "zip" => "application/zip",
        _ => "",
    }
    .to_string()
}

/// A file as it goes: refused when too big to cross (uploads.ts
/// `checkUploads`); a PNG or JPEG decoded for its preview.
pub fn pending(name: &str, bytes: Vec<u8>, id: u64) -> Result<Pending, String> {
    if bytes.len() > MAX_UPLOAD {
        let n = if name.is_empty() {
            "pasted image"
        } else {
            name
        };
        return Err(format!(
            "{n} is over {} MiB; ask an agent to attach it from disk",
            MAX_UPLOAD >> 20
        ));
    }
    let mime = mime_of(name);
    let preview = if mime == "image/png" || mime == "image/jpeg" {
        decode(bytes.clone()).ok()
    } else {
        None
    };
    Ok(Pending {
        name: name.to_string(),
        mime,
        bytes: bytes.into(),
        preview,
        id,
    })
}

/// Reads files from disk into pending files, each read's refusal kept.
pub fn read_files(paths: &[std::path::PathBuf], first_id: u64) -> Vec<Result<Pending, String>> {
    paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if p.metadata().is_ok_and(|m| m.len() as usize > MAX_UPLOAD) {
                return Err(format!(
                    "{name} is over {} MiB; ask an agent to attach it from disk",
                    MAX_UPLOAD >> 20
                ));
            }
            let bytes = std::fs::read(p).map_err(|e| format!("couldn't read {name}: {e}"))?;
            pending(&name, bytes, first_id + i as u64)
        })
        .collect()
}

/// The files waiting to go, each with an × to take it back off (or why a
/// paste or drop was refused).
pub fn pending_files(
    f: &mut Frame,
    key: &str,
    files: &[Pending],
    refused: Option<&str>,
    disabled: bool,
    remove: Arc<dyn Fn(&mut State, u64)>,
) -> Option<El> {
    let k = f.k;
    if files.is_empty() {
        return refused.map(|r| {
            text(r.to_string())
                .key(format!("{key}/refused"))
                .font(k.ty.small)
                .color(k.c.text_muted)
                .px(4.0)
                .wrap()
        });
    }
    let mut out = row()
        .key(key.to_string())
        .gap(8.0)
        .flex_wrap()
        .px(4.0)
        .label("Files to send");
    for p in files {
        let name = if p.name.is_empty() {
            "pasted image".to_string()
        } else {
            p.name.clone()
        };
        let id = p.id;
        let rm = remove.clone();
        let x = IconButton::new(&icons::X)
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Mini)
            .label(format!("Remove {name}"))
            .help("Remove")
            .disabled(disabled)
            .on_press(move |s: &mut State| rm(s, id))
            .build(k);
        let item = match &p.preview {
            Some(img) => {
                let (w, h) = img
                    .size()
                    .map_or((80.0, 80.0), |[w, h]| (w as f32, h as f32));
                let pw = (80.0 * w / h.max(1.0)).clamp(48.0, 160.0);
                el().w(pw)
                    .h(80.0)
                    .radius(6.0)
                    .border(1.0, k.c.border_base)
                    .clip_rounded(6.0)
                    .pixels_fit(img.pixels(), SurfaceFit::Fill)
                    .help(name)
                    .child(
                        el().absolute(pw - 24.0, 4.0)
                            .radius(10.0)
                            .fill(k.c.bg_base.alpha(0.85))
                            .border(1.0, k.c.border_base)
                            .child(x),
                    )
            }
            None => row()
                .h(32.0)
                .gap(6.0)
                .center_y()
                .px(8.0)
                .radius(6.0)
                .fill(k.c.layer_02)
                .border(1.0, k.c.border_base)
                .child(icon(file_kind_icon(&p.name, &p.mime), 16.0).color(k.c.icon_muted))
                .child(
                    text(name.clone())
                        .font(k.ty.small)
                        .color(k.c.text_base)
                        .truncate()
                        .max_w(192.0)
                        .help(name),
                )
                .child(x),
        };
        out = out.child(item.key(format!("{key}/f{id}")));
    }
    Some(out)
}

/// The clip: opens the file picker. With `why`, files are off here: the
/// clip shows disabled with the reason.
pub fn attach_button(
    f: &Frame,
    key: &str,
    why: Option<&str>,
    full: bool,
    on_pick: Arc<dyn Fn(&mut State, &mut Tx)>,
) -> El {
    let b = IconButton::new(&icons::PAPERCLIP)
        .variant(IconButtonVariant::Ghost)
        .control_size(ControlSize::Small);
    match why {
        Some(why) => b
            .label(format!("No files here: {why}"))
            .help(why.to_string())
            .disabled(true),
        None => b
            .label("Add files")
            .help("Add files (or drop them here, or paste a screenshot or file)")
            .disabled(full)
            .on_press_tx(move |s: &mut State, tx: &mut Tx| on_pick(s, tx)),
    }
    .build(f.k)
    .key(key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(name: &str, mime: &str) -> &'static str {
        file_kind_icon(name, mime).name
    }

    #[test]
    fn file_kind_goes_by_the_extension_any_case() {
        assert_eq!(kind("Shot.PNG", ""), "lucide-file-image");
        assert_eq!(kind("notes.md", ""), "lucide-file-text");
        assert_eq!(kind("data.csv", ""), "lucide-file-spreadsheet");
        assert_eq!(kind("main.rs", ""), "lucide-file-code");
        assert_eq!(kind("logs.tar.gz", ""), "lucide-file-archive");
    }

    #[test]
    fn file_kind_falls_back_to_the_mime_type_then_a_page() {
        assert_eq!(kind("", "image/png"), "lucide-file-image");
        assert_eq!(
            kind("blob", "text/plain; charset=utf-8"),
            "lucide-file-text"
        );
        assert_eq!(kind(".bashrc", ""), "lucide-file");
        assert_eq!(kind("thing.xyz", ""), "lucide-file");
    }

    #[test]
    fn sizes_and_kinds() {
        assert_eq!(human_size(900), "900 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(3 << 20), "3.0 MB");
        let f = |name: &str, mime: &str, size: i64| File {
            name: name.into(),
            mime: mime.into(),
            size,
            ..File::default()
        };
        assert!(looks_like_image(&f("a.png", "", 10)));
        assert!(looks_like_image(&f("x", "image/jpeg; q=1", 10)));
        assert!(!looks_like_image(&f("a.png", "application/pdf", 10)));
        assert!(!looks_like_image(&f("a.png", "", MAX_PREVIEW + 1)));
        assert!(can_open(&f("report.pdf", "", 1)));
        assert!(!can_open(&f("run.sh", "", 1)));
        assert_eq!(
            offline_line("the file is on Mac mini, which isn't connected now"),
            "On Mac mini, offline"
        );
        assert_eq!(offline_line("nope"), "Its device is offline");
    }

    #[test]
    fn uploads_are_checked() {
        assert!(
            pending("big.bin", vec![0; MAX_UPLOAD + 1], 1)
                .err()
                .unwrap()
                .contains("over 32 MiB")
        );
        let p = pending("a.txt", b"hi".to_vec(), 1).ok().unwrap();
        assert_eq!(p.upload().data, "aGk=");
        assert_eq!(
            pending("", vec![1], 2).ok().unwrap().upload().name,
            "pasted.png"
        );
    }
}

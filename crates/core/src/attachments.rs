//! Agent attachments (Go I:internal/core/attachments.go `AttachFiles`,
//! `GetAttachment`, `attachmentDst`, `resolveExisting`, `freeName`): the
//! Core's `tools::Attacher`, which the tools' attach argument and
//! attachment_get call.
//!
//! [`crate::Core::attach_files`] checks a path with the never-leave rules
//! ([`blob::check_attach_path`]), copies it into the attachment store
//! (`files/`, the store the window's `irori://` scheme reads) and offers it
//! to the person's other devices. [`crate::Core::get_attachment`] finds a
//! file on a message the reader may read or on a Board comment, fetches it
//! over the mesh when another device holds it, and writes it out with
//! [`attachment_dst`].
//!
//! Boards: `attachment_dst` keys on the reader, not on a board, so nothing
//! there routes by board. The comment lookup is the private board's, then
//! each shared board's; a file on a shared card is fetched on that board's
//! own connections when no device of this person holds it (v0.5 §5.5,
//! `crate::meshview`). Attaching a file to a shared card's comment offers
//! it in the board's log (the board coord's `holds_file`).

use std::fs;
use std::path::{Path, PathBuf};

use gocompat::filepath;

/// Go `ErrNoAttachment`'s text: no message or card the caller may read
/// carries the file.
pub const NO_ATTACHMENT: &str = "no message or card you can read carries that attachment";

/// Go `FileOfflineError`'s text.
pub(crate) fn offline_text(on: &str) -> String {
    format!("the file is on {on}, which isn't connected now; try again when it is")
}

/// What a fetch past [`crate::meshview::FETCH_TIMEOUT`] says (the
/// window's text for the same deadline).
pub(crate) const TIMED_OUT: &str =
    "the file took too long to arrive from the device that holds it; try again";

/// Go `strings.TrimSpace`.
pub(crate) fn trim_space(s: &str) -> &str {
    s.trim_matches(gocompat::text::is_space)
}

/// Go `filepath.Base` for the names attach uses.
pub(crate) fn base(p: &str) -> String {
    let t = p.trim_end_matches(['/', '\\']);
    if t.is_empty() {
        return if p.is_empty() {
            ".".into()
        } else {
            filepath::SEPARATOR.to_string()
        };
    }
    match t.rfind(['/', '\\']) {
        Some(i) => t[i + 1..].to_string(),
        None => t.to_string(),
    }
}

/// Go `filepath.EvalSymlinks` for a path that exists: canonical, without
/// Windows' `\\?\` prefix.
fn eval_symlinks(p: &Path) -> Option<PathBuf> {
    let c = fs::canonicalize(p).ok()?;
    let s = c.to_string_lossy();
    Some(if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        c
    })
}

/// Go `attachmentDst`: where a fetched file goes: `to` (a file, or an
/// existing folder to put `name` in), never inside the data dir, or the
/// reader's own folder under [`blob::RECEIVED_DIR`]. A relative `to` is
/// taken against `cwd()`, the reader's working directory.
pub(crate) fn attachment_dst(
    data_dir: &Path,
    reader: &str,
    name: &str,
    to: &str,
    cwd: impl FnOnce() -> String,
) -> Result<PathBuf, String> {
    let data = filepath::abs(&data_dir.to_string_lossy()).map_err(|e| e.to_string())?;
    let to = trim_space(to);
    if to.is_empty() {
        let mut folder = blob::safe_name(&reader.replace('/', "_"), "");
        if folder.is_empty() || folder.starts_with("attachment-") {
            folder = "unknown".into();
        }
        return Ok(PathBuf::from(filepath::join(&[
            data.as_str(),
            blob::RECEIVED_DIR,
            &folder,
            name,
        ])));
    }
    let mut to = to.to_string();
    if !filepath::is_abs(&to) {
        let cwd = cwd();
        if cwd.is_empty() || !filepath::is_abs(&cwd) {
            return Err(format!(
                "{} is relative and your working directory is unknown: give an absolute path",
                gocompat::quote::quote(&to)
            ));
        }
        to = filepath::join(&[cwd.as_str(), to.as_str()]);
    }
    if fs::metadata(&to).is_ok_and(|m| m.is_dir()) {
        to = filepath::join(&[to.as_str(), name]);
    }
    let to = filepath::clean(&to);
    // Resolve what exists of it, so a link can't point the write into the
    // data dir: the longest prefix that exists is resolved and the rest
    // (folders copy_out will make) joined back on, so a link above a
    // missing folder is followed too.
    let parent = resolve_existing(Path::new(&to).parent().unwrap_or(Path::new(&to)));
    let real_data = eval_symlinks(Path::new(&data))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| data.clone());
    if blob::within(&parent.to_string_lossy(), &real_data) || blob::within(&to, &data) {
        return Err(format!(
            "{to} is inside Irori's data dir; leave to empty for the default folder"
        ));
    }
    Ok(PathBuf::from(to))
}

/// Go `resolveExisting`: `p` with its longest existing prefix's symlinks
/// resolved and the missing rest joined back on.
pub(crate) fn resolve_existing(p: &Path) -> PathBuf {
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = p;
    loop {
        if let Some(mut real) = eval_symlinks(cur) {
            for r in rest.iter().rev() {
                real.push(r);
            }
            return real;
        }
        match (cur.parent(), cur.file_name()) {
            (Some(up), Some(name)) if up != cur && !up.as_os_str().is_empty() => {
                rest.push(name.to_os_string());
                cur = up;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// Go `freeName`: `dst`, or `dst` with " (2)", " (3)" ... before its
/// extension when another file already holds that name.
pub(crate) fn free_name(dst: &Path, sha: &str) -> PathBuf {
    if fs::symlink_metadata(dst).is_err() || blob::same_content(dst, sha) {
        return dst.to_path_buf();
    }
    let s = dst.to_string_lossy();
    let ext = go_ext(&s);
    let stem = &s[..s.len() - ext.len()];
    for i in 2.. {
        let p = PathBuf::from(format!("{stem} ({i}){ext}"));
        if fs::symlink_metadata(&p).is_err() || blob::same_content(&p, sha) {
            return p;
        }
    }
    unreachable!()
}

/// Go `filepath.Ext`.
fn go_ext(p: &str) -> &str {
    for (i, c) in p.char_indices().rev() {
        match c {
            '.' => return &p[i..],
            '/' | '\\' => break,
            _ => {}
        }
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(target, link)
        }
        #[cfg(not(windows))]
        {
            std::os::unix::fs::symlink(target, link)
        }
    }

    // Go TestAttachmentDstFollowsALinkAboveAMissingFolder: a link to the
    // data dir with a missing folder under it still counts as inside the
    // data dir: what exists of the path is resolved, not only its parent.
    #[test]
    fn attachment_dst_follows_a_link_above_a_missing_folder() {
        let (data, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let link = other.path().join("to-data");
        if let Err(e) = link_dir(data.path(), &link) {
            eprintln!("skip: can't make a symlink here: {e}");
            return;
        }
        let into = link.join("newdir").join("f.txt");
        assert!(
            attachment_dst(
                data.path(),
                "x/y",
                "f.txt",
                &into.to_string_lossy(),
                String::new
            )
            .is_err(),
            "a write through a link into the data dir was allowed"
        );
        let ok = other.path().join("newdir").join("f.txt");
        attachment_dst(
            data.path(),
            "x/y",
            "f.txt",
            &ok.to_string_lossy(),
            String::new,
        )
        .expect("an ordinary new folder");
    }

    #[test]
    fn attachment_dst_defaults_and_refusals() {
        let (data, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        // The reader's own folder under received/, its slashes made safe.
        let d = attachment_dst(data.path(), "dev1/sess", "plan.pdf", " ", String::new).unwrap();
        let want = Path::new(&filepath::abs(&data.path().to_string_lossy()).unwrap())
            .join(blob::RECEIVED_DIR)
            .join("dev1_sess")
            .join("plan.pdf");
        assert_eq!(d, want);
        // A reader with no usable name gets "unknown".
        let d = attachment_dst(data.path(), "", "a.txt", "", String::new).unwrap();
        assert!(
            d.ends_with(Path::new("received").join("unknown").join("a.txt")),
            "{d:?}"
        );
        // Inside the data dir is refused.
        let e = attachment_dst(
            data.path(),
            "r",
            "a.txt",
            &data.path().join("irori.json").to_string_lossy(),
            String::new,
        )
        .unwrap_err();
        assert!(
            e.ends_with("is inside Irori's data dir; leave to empty for the default folder"),
            "{e}"
        );
        // Relative needs a known working directory.
        let e = attachment_dst(data.path(), "r", "a.txt", "out/a.txt", String::new).unwrap_err();
        assert_eq!(
            e,
            "\"out/a.txt\" is relative and your working directory is unknown: give an absolute path"
        );
        let cwd = other.path().to_string_lossy().into_owned();
        let d = attachment_dst(data.path(), "r", "a.txt", "out", || cwd.clone()).unwrap();
        assert_eq!(
            d,
            PathBuf::from(filepath::clean(&other.path().join("out").to_string_lossy()))
        );
        // An existing folder takes the name.
        let d = attachment_dst(data.path(), "r", "a.txt", &cwd, String::new).unwrap();
        assert_eq!(
            d,
            PathBuf::from(filepath::clean(
                &other.path().join("a.txt").to_string_lossy()
            ))
        );
    }

    #[test]
    fn free_name_numbers_before_the_extension() {
        let dir = tempfile::tempdir().unwrap();
        let sha = "0".repeat(64);
        let p = dir.path().join("plan.pdf");
        assert_eq!(free_name(&p, &sha), p);
        fs::write(&p, b"x").unwrap();
        assert_eq!(free_name(&p, &sha), dir.path().join("plan (2).pdf"));
        fs::write(dir.path().join("plan (2).pdf"), b"y").unwrap();
        assert_eq!(free_name(&p, &sha), dir.path().join("plan (3).pdf"));
        assert_eq!(go_ext("a/b.c/d"), "");
        assert_eq!(go_ext(".bashrc"), ".bashrc");
    }
}

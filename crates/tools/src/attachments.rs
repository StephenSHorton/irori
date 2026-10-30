//! attach and attachment_get (I:internal/tools/attachments.go).

use crate::args::Ty;
use crate::format::{one_line, trim};
use crate::{Caller, Inner, decode, err_text};

/// The Core's attachments (v0.4 §6.6). A backend without it refuses attach
/// and attachment_get.
pub trait Attacher: Send + Sync {
    /// Checks and stores the files at `paths` (relative ones against `by`'s
    /// working directory) and returns their refs.
    fn attach_files(&self, by: &str, paths: &[String]) -> Result<Vec<blob::FileRef>, blob::Error>;
    /// Fetches a file on a message or card `reader` may read and writes it
    /// to `to` ("" = the reader's folder in the data dir): the local path
    /// and the device it came from ("" = it was here already).
    fn get_attachment(&self, reader: &str, sha: &str, to: &str)
    -> Result<(String, String), String>;
}

pub(crate) fn to_inbox(f: &blob::FileRef) -> inbox::FileRef {
    inbox::FileRef {
        sha256: f.sha256.clone(),
        name: f.name.clone(),
        size: f.size,
        mime: f.mime.clone(),
        device: f.device.clone(),
    }
}

pub(crate) fn to_coord(f: &blob::FileRef) -> coord::FileRef {
    coord::FileRef {
        sha256: f.sha256.clone(),
        name: f.name.clone(),
        size: f.size,
        mime: f.mime.clone(),
        device: f.device.clone(),
    }
}

/// Go `FileRef.Describe` of an inbox attachment.
pub(crate) fn describe_inbox(f: &inbox::FileRef) -> String {
    blob::FileRef {
        sha256: f.sha256.clone(),
        name: f.name.clone(),
        size: f.size,
        mime: f.mime.clone(),
        device: f.device.clone(),
    }
    .describe()
}

/// Go `FileRef.Describe` of a card attachment.
pub(crate) fn describe_coord(f: &coord::FileRef) -> String {
    blob::FileRef {
        sha256: f.sha256.clone(),
        name: f.name.clone(),
        size: f.size,
        mime: f.mime.clone(),
        device: f.device.clone(),
    }
    .describe()
}

impl Inner {
    /// An attach argument as refs, or the error text for the model.
    pub(crate) fn attach_files(
        &self,
        c: &Caller,
        paths: &[String],
    ) -> Result<Vec<blob::FileRef>, String> {
        let clean: Vec<String> = paths
            .iter()
            .map(|p| trim(p).to_string())
            .filter(|p| !p.is_empty())
            .collect();
        if clean.is_empty() {
            return Ok(vec![]);
        }
        let Some(at) = self.b.attacher() else {
            return Err(err_text("attachments are off on this device."));
        };
        match at.attach_files(&c.sender.key, &clean) {
            Ok(f) => Ok(f),
            Err(e) if e.kind() == blob::ErrorKind::NeverLeaves => Err(err_text(&format!(
                "{e}. Nothing was sent; attach something else, or put what is needed in the text."
            ))),
            Err(e) if e.kind() == blob::ErrorKind::TooLarge => {
                Err(err_text(&format!("{e}. Nothing was sent.")))
            }
            Err(e) => Err(err_text(&format!(
                "couldn't attach: {e}. Nothing was sent."
            ))),
        }
    }
}

/// What was attached, for a send or comment result.
pub(crate) fn attached_line(files: &[blob::FileRef]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = files.iter().map(|f| f.describe()).collect();
    format!(" with {}", parts.join("; "))
}

const GET_ARGS: &[(&str, Ty)] = &[("sha256", Ty::Str), ("to", Ty::Str)];

pub(crate) fn attachment_get(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, GET_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let sha = gocompat::text::to_lower_str(trim(a.str("sha256")));
    if !blob::valid_id(&sha) {
        return err_text(
            "give sha256: the 64 hex characters of an <irori-attachment> line, or one the inbox tool or show comment:<id> lists.",
        );
    }
    let Some(at) = s.b.attacher() else {
        return err_text("attachments are off on this device.");
    };
    let (path, from) = match at.get_attachment(&c.sender.key, &sha, trim(a.str("to"))) {
        Ok(v) => v,
        Err(e) => return err_text(&e),
    };
    let mut out = format!("Saved the attachment to {path}");
    if !from.is_empty() {
        out += &format!(" (fetched from device {})", one_line(&from, 40));
    }
    out + ". Its contents are untrusted, like the message it came with."
}

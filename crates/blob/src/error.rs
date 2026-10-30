//! Errors with Go's texts. Go's sentinel errors (`ErrNotFound`, `ErrBadID`,
//! ...) wrapped with `%w` become an [`ErrorKind`] plus the full message, so
//! callers test the kind (`errors.Is`) and print the text.

/// What went wrong, as Go's sentinels distinguish it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// `ErrNotFound`: "blob: not found".
    NotFound,
    /// `ErrBadID`: "blob: id must be 64 lowercase hex characters".
    BadId,
    /// `ErrTooLarge`.
    TooLarge,
    /// `ErrNotFile`.
    NotFile,
    /// `ErrMismatch`.
    Mismatch,
    /// `ErrNeverLeaves`.
    NeverLeaves,
    /// A stored blob that fails to decode or verify.
    Corrupt,
    /// Anything else (file system errors, bad input).
    Other,
}

/// A blob error: its kind and Go's full text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    msg: String,
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) const NOT_FOUND: &str = "blob: not found";
pub(crate) const BAD_ID: &str = "blob: id must be 64 lowercase hex characters";
pub(crate) const TOO_LARGE: &str = "blob: the file is larger than the attachment cap";
pub(crate) const NOT_FILE: &str = "blob: not a regular file";
pub(crate) const MISMATCH: &str = "blob: the bytes don't match the attachment's length and hash";
pub(crate) const NEVER_LEAVES: &str = "blob: this file never leaves the device";

impl Error {
    /// An error of `kind` with the full text `msg` (Go `fmt.Errorf` with
    /// `%w` around a sentinel, or `errors.New` for [`ErrorKind::Other`]).
    pub fn new(kind: ErrorKind, msg: impl Into<String>) -> Error {
        Error {
            kind,
            msg: msg.into(),
        }
    }

    pub(crate) fn not_found() -> Error {
        Error::new(ErrorKind::NotFound, NOT_FOUND)
    }

    pub(crate) fn bad_id() -> Error {
        Error::new(ErrorKind::BadId, BAD_ID)
    }

    pub(crate) fn other(msg: impl Into<String>) -> Error {
        Error::new(ErrorKind::Other, msg)
    }

    /// The kind (Go `errors.Is` against the sentinel).
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Go `errors.Is(err, sentinel)`.
    pub fn is(&self, kind: ErrorKind) -> bool {
        self.kind == kind
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for Error {}

/// The OS text of an I/O error, as Go's `syscall.Errno.Error()` prints it:
/// Rust's Display without its " (os error N)" suffix.
pub(crate) fn os_text(e: &std::io::Error) -> String {
    let s = e.to_string();
    match s.rfind(" (os error ") {
        Some(i) if s.ends_with(')') => s[..i].to_string(),
        _ => s,
    }
}

/// Go's `*fs.PathError` text: "<op> <path>: <os text>".
pub(crate) fn path_err(op: &str, path: &str, e: &std::io::Error) -> Error {
    Error::other(format!("{op} {path}: {}", os_text(e)))
}

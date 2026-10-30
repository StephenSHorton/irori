//! coord's errors: Go's sentinel errors wrapped with `%w: detail`, kept
//! matchable like `errors.Is` (coord.md §5 "Errors").

use std::fmt;

/// Which Go sentinel an error wraps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    NotFound,
    Held,
    Ambiguous,
    BadResource,
    BadState,
    Unimplemented,
    NotAllowed,
    Invalid,
    /// `ErrSubtask`, which wraps `ErrInvalid`.
    Subtask,
    /// A store error (Go `fmt.Errorf("coord: %w", err)`).
    Store,
}

impl ErrorKind {
    /// The sentinel's own text.
    pub fn text(self) -> &'static str {
        match self {
            ErrorKind::NotFound => "coord: no such task or lease",
            ErrorKind::Held => "coord: held by someone else",
            ErrorKind::Ambiguous => "coord: that short ref names more than one card",
            ErrorKind::BadResource => {
                "coord: resource must be path:<repoKey>:<glob> or a name like gpu/qwen"
            }
            ErrorKind::BadState => "coord: not a task state",
            ErrorKind::Unimplemented => "coord: not implemented yet",
            ErrorKind::NotAllowed => "coord: only the holder, the creator or a person may do that",
            ErrorKind::Invalid => "coord: invalid argument",
            ErrorKind::Subtask => "coord: invalid argument: subtasks",
            ErrorKind::Store => "coord",
        }
    }
}

/// A coord error: its kind and Go's full message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    msg: String,
    store: Option<store::ErrorKind>,
}

impl Error {
    /// `fmt.Errorf("%w: detail", kind)`.
    pub fn new(kind: ErrorKind, detail: impl fmt::Display) -> Error {
        Error {
            kind,
            msg: format!("{}: {detail}", kind.text()),
            store: None,
        }
    }

    /// The sentinel alone.
    pub fn bare(kind: ErrorKind) -> Error {
        Error {
            kind,
            msg: kind.text().into(),
            store: None,
        }
    }

    /// A message built by the caller around the sentinel's text.
    pub fn raw(kind: ErrorKind, msg: impl Into<String>) -> Error {
        Error {
            kind,
            msg: msg.into(),
            store: None,
        }
    }

    /// `fmt.Errorf("prefix: %w", e)`.
    pub fn prefixed(self, prefix: impl fmt::Display) -> Error {
        Error {
            msg: format!("{prefix}: {}", self.msg),
            ..self
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// `errors.Is(err, kind)`: ErrSubtask is also ErrInvalid.
    pub fn is(&self, kind: ErrorKind) -> bool {
        self.kind == kind || (kind == ErrorKind::Invalid && self.kind == ErrorKind::Subtask)
    }

    /// The store error's kind, for a Store error.
    pub fn store_kind(&self) -> Option<store::ErrorKind> {
        self.store
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for Error {}

impl From<store::Error> for Error {
    fn from(e: store::Error) -> Error {
        Error {
            kind: ErrorKind::Store,
            msg: format!("coord: {e}"),
            store: Some(e.kind()),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Shorthands for the common sentinels.
pub(crate) fn invalid(detail: impl fmt::Display) -> Error {
    Error::new(ErrorKind::Invalid, detail)
}
pub(crate) fn not_found(detail: impl fmt::Display) -> Error {
    Error::new(ErrorKind::NotFound, detail)
}
pub(crate) fn not_allowed(detail: impl fmt::Display) -> Error {
    Error::new(ErrorKind::NotAllowed, detail)
}
pub(crate) fn bad_state(detail: impl fmt::Display) -> Error {
    Error::new(ErrorKind::BadState, detail)
}
pub(crate) fn held(detail: impl fmt::Display) -> Error {
    Error::new(ErrorKind::Held, detail)
}
pub(crate) fn subtask(detail: impl fmt::Display) -> Error {
    Error::new(ErrorKind::Subtask, detail)
}

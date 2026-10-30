//! Store errors (store.md §1.2): one type whose [`ErrorKind`] stands for
//! the Go sentinel a caller would match with `errors.Is`, and whose text is
//! the full Go message, wrapping included.

use std::fmt;

/// The Go sentinel an error wraps. Match with [`Error::is`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// `ErrLocalKind`: a device-local kind given to Append or Insert.
    LocalKind,
    /// `ErrNotLocal`: AppendLocal given a kind that replicates.
    NotLocal,
    /// `ErrNoCert`: the row's device has no cert yet; hold and retry.
    NoCert,
    /// `ErrBadSig`: the signature does not verify.
    BadSig,
    /// `ErrUnsigned`: a replicated row with no signature.
    Unsigned,
    /// `ErrPastCut`: above its device's revocation cut.
    PastCut,
    /// `ErrFork`: a different signed row for a held (device, seq).
    Fork,
    /// `ErrHLCOrder`: the HLC does not follow the device's previous row.
    HlcOrder,
    /// `ErrAhead`: stamped too far past local wall time; retry later.
    Ahead,
    /// `ErrRestoring`: appends wait for this device's own rows.
    Restoring,
    /// `ErrOwnDevice`: own id inserted without a Verifier.
    OwnDevice,
    /// `ErrGap`: a seq past the next expected one.
    Gap,
    /// `ErrClosed`.
    Closed,
    /// `ErrNotBoardKind`: not on a board log's allow-list.
    NotBoardKind,
    /// `ErrBoardActor`: not a board actor.
    BoardActor,
    /// `ErrOutsideBoard`: names something the board does not hold.
    OutsideBoard,
    /// SQLite reported an error.
    Sqlite,
    /// A file operation failed.
    Io,
    /// Anything else (Go `errors.New` / `fmt.Errorf` without a sentinel).
    Other,
}

impl ErrorKind {
    /// The sentinel's Go text (empty for the kinds without one).
    pub fn text(self) -> &'static str {
        match self {
            ErrorKind::LocalKind => "store: device-local kind; use AppendLocal",
            ErrorKind::NotLocal => "store: kind replicates; use Append",
            ErrorKind::NoCert => "store: row's device is not certified",
            ErrorKind::BadSig => "store: row signature does not verify",
            ErrorKind::Unsigned => "store: row is unsigned",
            ErrorKind::PastCut => "store: row is past its device's revocation cut",
            ErrorKind::Fork => "store: device signed two different rows for one seq",
            ErrorKind::HlcOrder => "store: row's hlc does not follow its device's previous row",
            ErrorKind::Ahead => "store: row is stamped too far in the future",
            ErrorKind::Restoring => "store: restoring this device's rows; appends wait",
            ErrorKind::OwnDevice => {
                "store: refusing to insert a row for this device's own id; use Append"
            }
            ErrorKind::Gap => "store: replicated row leaves a seq gap",
            ErrorKind::Closed => "store: closed",
            ErrorKind::NotBoardKind => "store: kind is not allowed in a board log",
            ErrorKind::BoardActor => "store: actor is not allowed in a board log",
            ErrorKind::OutsideBoard => "store: row references something outside this board",
            ErrorKind::Sqlite | ErrorKind::Io | ErrorKind::Other => "",
        }
    }
}

/// A store error: its sentinel and its full message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    msg: String,
}

/// `Result<T, store::Error>`.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// An error of `kind` with exactly this message. Verifiers use it to
    /// return `NoCert` / `BadSig` / `BoardActor` with their own text.
    pub fn new(kind: ErrorKind, msg: impl Into<String>) -> Error {
        Error {
            kind,
            msg: msg.into(),
        }
    }

    /// The bare sentinel (Go `return ErrX`).
    pub fn sentinel(kind: ErrorKind) -> Error {
        Error::new(kind, kind.text())
    }

    /// Go `fmt.Errorf("%w: <detail>", ErrX)`.
    pub fn wrap(kind: ErrorKind, detail: impl fmt::Display) -> Error {
        Error::new(kind, format!("{}: {detail}", kind.text()))
    }

    /// An error with no sentinel.
    pub fn other(msg: impl Into<String>) -> Error {
        Error::new(ErrorKind::Other, msg)
    }

    /// The sentinel this error wraps.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Go `errors.Is(err, ErrX)`.
    pub fn is(&self, kind: ErrorKind) -> bool {
        self.kind == kind
    }

    /// Go `fmt.Errorf("<prefix>%w", err)`: the same sentinel, more text.
    pub fn context(self, prefix: impl fmt::Display) -> Error {
        Error {
            kind: self.kind,
            msg: format!("{prefix}{}", self.msg),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Error {
        Error::new(ErrorKind::Sqlite, e.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::new(ErrorKind::Io, e.to_string())
    }
}

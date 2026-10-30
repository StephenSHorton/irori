//! Go `Strict` (`strict.go`): the stricter pass Share to board runs after
//! [`crate::Scrubber::string`]. It drops absolute paths and home
//! directories from text that is about to reach other people. Relative
//! paths and URLs stay.

use std::sync::LazyLock;

use regex::Regex;

/// What [`strict`] puts where a path was.
pub const PATH_MARKER: &str = "[path]";

const PATH_START: &str = r#"[\s"'(\[=,:*`]"#;
const PATH_REST: &str = r#"[^\s"'<>|*?)\]`]*"#;

fn sources() -> Vec<String> {
    vec![
        format!(r"(?i)(^|[^A-Za-z0-9])file://{PATH_REST}"),
        format!(r"(^|[^A-Za-z0-9])[A-Za-z]:[\\/]{PATH_REST}"),
        format!(r#"(^|{PATH_START})\\\\[^\s"'<>|*?\\)\]`]+(?:\\{PATH_REST})*"#),
        format!(
            r"(^|{PATH_START})(?:~|\$HOME|\$\{{HOME\}}|%USERPROFILE%|%HOMEPATH%)(?:[\\/]{PATH_REST})?"
        ),
        format!(r"(^|{PATH_START})/[A-Za-z0-9._@+-]+/{PATH_REST}"),
    ]
}

static STRICT_PATHS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    sources()
        .iter()
        .map(|p| gocompat::regex::from_go(p).unwrap_or_else(|e| panic!("strict {p:?}: {e}")))
        .collect()
});

/// Go `Strict`: every absolute path and home directory in `input` becomes
/// [`PATH_MARKER`], keeping the character before it.
pub fn strict(input: &str) -> String {
    let mut out = input.to_string();
    for re in STRICT_PATHS.iter() {
        out = re
            .replace_all(&out, format!("${{1}}{PATH_MARKER}").as_str())
            .into_owned();
    }
    out
}

/// Go `StrictDropped`: whether [`strict`] would change `input`.
pub fn strict_dropped(input: &str) -> bool {
    strict(input) != input
}

pub(crate) fn go_patterns() -> Vec<(&'static str, String)> {
    let names = [
        "strictPaths[0]",
        "strictPaths[1]",
        "strictPaths[2]",
        "strictPaths[3]",
        "strictPaths[4]",
    ];
    names.into_iter().zip(sources()).collect()
}

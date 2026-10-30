//! Go `flag.FlagSet` with `ContinueOnError` (surface.md §7.5): `-f`,
//! `--f`, `-f=v`, `--f v`; a bool flag takes no separate value; parsing
//! stops at the first non-flag argument, at a lone `-`, or after `--`.
//! The error texts and `PrintDefaults` are Go's, byte for byte.

/// One flag's value slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Str(String),
}

#[derive(Debug, Clone)]
struct Flag {
    name: String,
    usage: String,
    default: Value,
    value: Value,
}

/// A set of flags for one command (Go `flag.NewFlagSet(name,
/// ContinueOnError)`).
#[derive(Debug, Clone)]
pub struct FlagSet {
    name: String,
    flags: Vec<Flag>,
    args: Vec<String>,
}

/// Go `flag.ErrHelp`'s text.
pub const ERR_HELP: &str = "flag: help requested";

impl FlagSet {
    pub fn new(name: &str) -> FlagSet {
        FlagSet {
            name: name.to_string(),
            flags: Vec::new(),
            args: Vec::new(),
        }
    }

    fn define(&mut self, name: &str, default: Value, usage: &str) {
        self.flags.push(Flag {
            name: name.to_string(),
            usage: usage.to_string(),
            default: default.clone(),
            value: default,
        });
    }

    pub fn bool(&mut self, name: &str, default: bool, usage: &str) {
        self.define(name, Value::Bool(default), usage);
    }

    pub fn int(&mut self, name: &str, default: i64, usage: &str) {
        self.define(name, Value::Int(default), usage);
    }

    pub fn string(&mut self, name: &str, default: &str, usage: &str) {
        self.define(name, Value::Str(default.to_string()), usage);
    }

    fn get(&self, name: &str) -> &Value {
        &self
            .flags
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("flag: no flag -{name} defined"))
            .value
    }

    pub fn get_bool(&self, name: &str) -> bool {
        matches!(self.get(name), Value::Bool(true))
    }

    pub fn get_int(&self, name: &str) -> i64 {
        match self.get(name) {
            Value::Int(n) => *n,
            _ => 0,
        }
    }

    pub fn get_string(&self, name: &str) -> String {
        match self.get(name) {
            Value::Str(s) => s.clone(),
            _ => String::new(),
        }
    }

    /// The arguments left after the flags (Go `fs.Args()`).
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Go `fs.Parse(args)`. The error is Go's; Go's `ContinueOnError` also
    /// writes it and [`FlagSet::usage`] to the set's output, which the
    /// caller does (`-h`/`-help` writes only the usage).
    pub fn parse(&mut self, args: &[String]) -> Result<(), String> {
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if a.len() < 2 || !a.starts_with('-') {
                break;
            }
            let mut name = &a[1..];
            if let Some(rest) = name.strip_prefix('-') {
                if rest.is_empty() {
                    i += 1; // "--" ends the flags and is consumed
                    break;
                }
                name = rest;
            }
            if name.is_empty() || name.starts_with('-') || name.starts_with('=') {
                return Err(format!("bad flag syntax: {a}"));
            }
            i += 1;
            let (name, value) = match name.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (name.to_string(), None),
            };
            let Some(idx) = self.flags.iter().position(|f| f.name == name) else {
                if name == "help" || name == "h" {
                    return Err(ERR_HELP.into());
                }
                return Err(format!("flag provided but not defined: -{name}"));
            };
            let f = &mut self.flags[idx];
            match &mut f.value {
                Value::Bool(b) => match value {
                    None => *b = true,
                    Some(v) => match parse_bool(&v) {
                        Some(x) => *b = x,
                        None => {
                            return Err(format!(
                                "invalid boolean value {} for -{name}: parse error",
                                crate::quote::quote(&v)
                            ));
                        }
                    },
                },
                slot => {
                    let v = match value {
                        Some(v) => v,
                        None if i < args.len() => {
                            i += 1;
                            args[i - 1].clone()
                        }
                        None => return Err(format!("flag needs an argument: -{name}")),
                    };
                    match slot {
                        Value::Int(n) => match parse_int(&v) {
                            Ok(x) => *n = x,
                            Err(why) => {
                                return Err(format!(
                                    "invalid value {} for flag -{name}: {why}",
                                    crate::quote::quote(&v)
                                ));
                            }
                        },
                        Value::Str(s) => *s = v,
                        Value::Bool(_) => unreachable!(),
                    }
                }
            }
        }
        self.args = args[i..].to_vec();
        Ok(())
    }

    /// Go's default usage: `Usage of <name>:\n` then `PrintDefaults`.
    pub fn usage(&self) -> String {
        let mut out = format!("Usage of {}:\n", self.name);
        let mut fs: Vec<&Flag> = self.flags.iter().collect();
        fs.sort_by(|a, b| a.name.cmp(&b.name));
        for f in fs {
            let mut b = format!("  -{}", f.name);
            let (tname, usage) = unquote_usage(f);
            if !tname.is_empty() {
                b.push(' ');
                b.push_str(&tname);
            }
            if b.len() <= 4 {
                b.push('\t');
            } else {
                b.push_str("\n    \t");
            }
            b.push_str(&usage.replace('\n', "\n    \t"));
            let zero = match &f.default {
                Value::Bool(v) => !*v,
                Value::Int(v) => *v == 0,
                Value::Str(v) => v.is_empty(),
            };
            if !zero {
                match &f.default {
                    Value::Str(s) => b.push_str(&format!(" (default {})", crate::quote::quote(s))),
                    Value::Int(n) => b.push_str(&format!(" (default {n})")),
                    Value::Bool(v) => b.push_str(&format!(" (default {v})")),
                }
            }
            out.push_str(&b);
            out.push('\n');
        }
        out
    }
}

/// Go `flag.UnquoteUsage`: a back-quoted word in the usage names the
/// value; else the type's name (`""` for a bool).
fn unquote_usage(f: &Flag) -> (String, String) {
    let u = &f.usage;
    if let Some(s) = u.find('`')
        && let Some(e) = u[s + 1..].find('`')
    {
        let name = u[s + 1..s + 1 + e].to_string();
        let usage = format!("{}{}{}", &u[..s], name, &u[s + 1 + e + 1..]);
        return (name, usage);
    }
    let t = match f.value {
        Value::Bool(_) => "",
        Value::Int(_) => "int",
        Value::Str(_) => "string",
    };
    (t.to_string(), u.clone())
}

/// Go `strconv.ParseBool`.
pub fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "1" | "t" | "T" | "TRUE" | "true" | "True" => Some(true),
        "0" | "f" | "F" | "FALSE" | "false" | "False" => Some(false),
        _ => None,
    }
}

/// Go `strconv.ParseInt(s, 0, 64)` as `flag` reports it: `Err("parse
/// error")` or `Err("value out of range")`.
pub fn parse_int(s: &str) -> Result<i64, &'static str> {
    const SYNTAX: &str = "parse error";
    const RANGE: &str = "value out of range";
    let (neg, body) = match s.as_bytes().first() {
        Some(b'+') => (false, &s[1..]),
        Some(b'-') => (true, &s[1..]),
        _ => (false, s),
    };
    if body.is_empty() {
        return Err(SYNTAX);
    }
    let lower = body.to_ascii_lowercase();
    let (base, digits) = if let Some(r) = lower.strip_prefix("0x") {
        (16, r.to_string())
    } else if let Some(r) = lower.strip_prefix("0b") {
        (2, r.to_string())
    } else if let Some(r) = lower.strip_prefix("0o") {
        (8, r.to_string())
    } else if lower.len() > 1 && lower.starts_with('0') {
        (8, lower[1..].to_string())
    } else {
        (10, lower.clone())
    };
    // Base 0 allows underscores between digits (and after a prefix).
    if digits.contains('_') && !underscore_ok(&lower) {
        return Err(SYNTAX);
    }
    let digits: String = digits.chars().filter(|&c| c != '_').collect();
    if digits.is_empty() {
        return Err(SYNTAX);
    }
    let mut n: u128 = 0;
    for c in digits.chars() {
        let d = c.to_digit(base).ok_or(SYNTAX)?;
        n = n * u128::from(base) + u128::from(d);
        if n > u128::from(u64::MAX) + 1 {
            n = u128::from(u64::MAX) + 1; // keep scanning for syntax
        }
    }
    if neg {
        if n > (1u128 << 63) {
            return Err(RANGE);
        }
        Ok((-(n as i128)) as i64)
    } else {
        if n > i64::MAX as u128 {
            return Err(RANGE);
        }
        Ok(n as i64)
    }
}

/// Go `strconv.underscoreOK` on the unsigned, lower-cased text.
fn underscore_ok(s: &str) -> bool {
    let b = s.as_bytes();
    let mut saw = '^';
    let mut i = 0;
    let mut hex = false;
    if b.len() >= 2 && b[0] == b'0' && matches!(b[1], b'x' | b'b' | b'o') {
        hex = b[1] == b'x';
        i = 2;
        saw = '0';
    }
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_digit() || (hex && c.is_ascii_hexdigit()) {
            saw = '0';
        } else if c == b'_' {
            if saw != '0' {
                return false;
            }
            saw = '_';
        } else {
            if saw == '_' {
                return false;
            }
            saw = '!';
        }
        i += 1;
    }
    saw != '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn events_set() -> FlagSet {
        let mut fs = FlagSet::new("events");
        fs.bool("follow", false, "keep streaming new rows");
        fs.string(
            "kinds",
            "",
            "comma list of kinds or prefixes ending in \".\" (agent.,device.started)",
        );
        fs.bool("json", false, "print raw NDJSON rows");
        fs.int(
            "limit",
            20,
            "recent rows to print first (0 = none with --follow)",
        );
        fs.int(
            "since",
            -1,
            "with --follow: replay this device's rows after this seq instead of the recent rows",
        );
        fs
    }

    #[test]
    fn parses_like_go() {
        let mut fs = events_set();
        fs.parse(&args(&[
            "--follow",
            "-kinds=agent.",
            "--limit",
            "5",
            "-json=false",
            "rest",
            "-x",
        ]))
        .unwrap();
        assert!(fs.get_bool("follow"));
        assert!(!fs.get_bool("json"));
        assert_eq!(fs.get_string("kinds"), "agent.");
        assert_eq!(fs.get_int("limit"), 5);
        assert_eq!(fs.get_int("since"), -1);
        assert_eq!(fs.args(), &args(&["rest", "-x"])[..]);

        let mut fs = events_set();
        fs.parse(&args(&["--", "-follow"])).unwrap();
        assert!(!fs.get_bool("follow"));
        assert_eq!(fs.args(), &args(&["-follow"])[..]);
    }

    #[test]
    fn errors_like_go() {
        for (a, want) in [
            (&["-bogus"][..], "flag provided but not defined: -bogus"),
            (&["-limit"], "flag needs an argument: -limit"),
            (
                &["-limit", "x"],
                "invalid value \"x\" for flag -limit: parse error",
            ),
            (
                &["-limit=99999999999999999999"],
                "invalid value \"99999999999999999999\" for flag -limit: value out of range",
            ),
            (
                &["-follow=yes"],
                "invalid boolean value \"yes\" for -follow: parse error",
            ),
            (&["---x"], "bad flag syntax: ---x"),
            (&["-=x"], "bad flag syntax: -=x"),
            (&["-h"], ERR_HELP),
            (&["--help"], ERR_HELP),
        ] {
            assert_eq!(events_set().parse(&args(a)), Err(want.to_string()), "{a:?}");
        }
    }

    #[test]
    fn ints_like_strconv() {
        assert_eq!(parse_int("0x1F"), Ok(31));
        assert_eq!(parse_int("017"), Ok(15));
        assert_eq!(parse_int("0b101"), Ok(5));
        assert_eq!(parse_int("-0o7"), Ok(-7));
        assert_eq!(parse_int("1_000"), Ok(1000));
        assert_eq!(parse_int("0_17"), Ok(15));
        assert_eq!(parse_int("1__0"), Err("parse error"));
        assert_eq!(parse_int("_1"), Err("parse error"));
        for bad in ["1_", "0x_", "0_", "08"] {
            assert_eq!(parse_int(bad), Err("parse error"), "{bad}");
        }
        assert_eq!(parse_int("0b_1"), Ok(1));
        assert_eq!(parse_int("0x_1_0"), Ok(16));
        assert_eq!(parse_int("-9223372036854775808"), Ok(i64::MIN));
        assert_eq!(parse_int("9223372036854775808"), Err("value out of range"));
        assert_eq!(parse_int(""), Err("parse error"));
        assert_eq!(parse_int("+"), Err("parse error"));
    }

    #[test]
    fn usage_like_go() {
        assert_eq!(
            events_set().usage(),
            "Usage of events:\n  -follow\n    \tkeep streaming new rows\n  -json\n    \tprint raw NDJSON rows\n  -kinds string\n    \tcomma list of kinds or prefixes ending in \".\" (agent.,device.started)\n  -limit int\n    \trecent rows to print first (0 = none with --follow) (default 20)\n  -since int\n    \twith --follow: replay this device's rows after this seq instead of the recent rows (default -1)\n"
        );
    }
}

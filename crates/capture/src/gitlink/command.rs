//! I:internal/capture/gitlink/command.go: does a shell command line make
//! commits?

use crate::gopath::slash_base;

/// git subcommands that can move HEAD onto new commits.
const COMMIT_SUBCOMMANDS: &[&str] = &[
    "commit",
    "merge",
    "cherry-pick",
    "revert",
    "rebase",
    "am",
    "pull",
];

/// git global options that take the next word as their value.
const GIT_ARG_OPTIONS: &[&str] = &[
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--super-prefix",
    "--config-env",
];

/// Words that run the rest of the line as a command.
const WRAPPERS: &[&str] = &[
    "sudo", "env", "command", "time", "nohup", "exec", "xargs", "builtin", "timeout", "nice", "{",
    "}", "!", "if", "then", "else", "elif", "do", "while", "until",
];

fn wrapper_values(name: &str) -> &'static [&'static str] {
    match name {
        "sudo" => &[
            "-u",
            "-g",
            "-C",
            "-D",
            "-h",
            "-p",
            "-r",
            "-t",
            "-U",
            "-T",
            "--user",
            "--group",
            "--chdir",
            "--host",
            "--prompt",
            "--role",
            "--type",
            "--other-user",
            "--close-from",
            "--command-timeout",
        ],
        "env" => &["-u", "-C", "--unset", "--chdir"],
        "nice" => &["-n", "--adjustment"],
        "timeout" => &["-s", "-k", "--signal", "--kill-after"],
        "xargs" => &[
            "-a",
            "-d",
            "-E",
            "-I",
            "-L",
            "-n",
            "-P",
            "-s",
            "--arg-file",
            "--delimiter",
            "--max-args",
            "--max-procs",
            "--max-chars",
        ],
        "time" => &["-f", "-o", "--format", "--output"],
        _ => &[],
    }
}

fn wrapper_positional(name: &str) -> usize {
    if name == "timeout" { 1 } else { 0 }
}

const SHELLS: &[&str] = &["bash", "sh", "zsh", "dash"];

/// Options of the commit-making subcommands whose value is the next word.
const COMMIT_VALUE_OPTIONS: &[&str] = &[
    "-m",
    "-F",
    "-C",
    "-c",
    "-t",
    "-s",
    "-X",
    "-x",
    "--message",
    "--file",
    "--template",
    "--author",
    "--date",
    "--strategy",
    "--strategy-option",
    "--onto",
    "--exec",
    "--reuse-message",
    "--reedit-message",
    "--fixup",
    "--squash",
    "--cleanup",
    "--mainline",
];

/// Whether a Bash command line can create commits: git commit / merge /
/// cherry-pick / revert / rebase / am / pull, or gh pr merge, anywhere in a
/// compound command, allowing `git -C <dir>` and other global options
/// before the subcommand; `bash -c` / `sh -c` arguments and `eval` are
/// parsed again (depth <= 4). Heredoc bodies are not recognized.
pub fn is_commit_command(command: &str) -> bool {
    check_line(command, 0)
}

fn check_line(line: &str, depth: usize) -> bool {
    if depth > 4 {
        return false;
    }
    split_commands(line)
        .iter()
        .any(|words| check_words(words, depth))
}

fn check_words(words: &[String], depth: usize) -> bool {
    let mut w: &[String] = words;
    loop {
        let Some(first) = w.first() else {
            return false;
        };
        let name = prog_name(first);
        if is_assignment(first) {
            w = &w[1..];
        } else if WRAPPERS.contains(&name.as_str()) {
            w = skip_wrapper_args(&name, &w[1..]);
        } else {
            break;
        }
    }
    let prog = prog_name(&w[0]);
    match prog.as_str() {
        "git" => is_git_commit(&w[1..]),
        "gh" => is_gh_merge(&w[1..]),
        "eval" => check_line(&w[1..].join(" "), depth + 1),
        p if SHELLS.contains(&p) => {
            for i in 1..w.len() {
                if w[i].starts_with('-')
                    && !w[i].starts_with("--")
                    && w[i].contains('c')
                    && i + 1 < w.len()
                {
                    return check_line(&w[i + 1], depth + 1);
                }
            }
            false
        }
        _ => false,
    }
}

fn skip_wrapper_args<'a>(name: &str, mut w: &'a [String]) -> &'a [String] {
    while let Some(opt) = w.first() {
        if !opt.starts_with('-') {
            break;
        }
        w = &w[1..];
        if opt == "--" {
            break;
        }
        if wrapper_values(name).contains(&opt.as_str()) && !w.is_empty() {
            w = &w[1..];
        }
    }
    let mut n = wrapper_positional(name);
    while n > 0 && !w.is_empty() {
        w = &w[1..];
        n -= 1;
    }
    w
}

fn is_git_commit(mut w: &[String]) -> bool {
    while let Some(opt) = w.first() {
        if !opt.starts_with('-') {
            break;
        }
        w = &w[1..];
        if GIT_ARG_OPTIONS.contains(&opt.as_str()) && !w.is_empty() {
            w = &w[1..];
        }
    }
    let Some(sub) = w.first() else {
        return false;
    };
    if !COMMIT_SUBCOMMANDS.contains(&sub.as_str()) {
        return false;
    }
    let args = &w[1..];
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--" {
            return true;
        }
        if a == "--abort" || a == "--quit" || a == "-h" || a == "--help" {
            return false;
        }
        if COMMIT_VALUE_OPTIONS.contains(&a) {
            i += 1;
        }
        i += 1;
    }
    true
}

fn is_gh_merge(w: &[String]) -> bool {
    let mut pos: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < w.len() && pos.len() < 2 {
        let a = w[i].as_str();
        if a == "-R" || a == "--repo" {
            i += 1;
        } else if !a.starts_with('-') {
            pos.push(a);
        }
        i += 1;
    }
    pos.len() == 2 && pos[0] == "pr" && pos[1] == "merge"
}

/// A command word's program name: base name, lowercased, without ".exe".
fn prog_name(word: &str) -> String {
    let w = word.replace('\\', "/");
    let name = gocompat::text::to_lower_str(&slash_base(&w));
    name.strip_suffix(".exe")
        .map(str::to_string)
        .unwrap_or(name)
}

fn is_assignment(word: &str) -> bool {
    let b = word.as_bytes();
    let Some(eq) = b.iter().position(|&c| c == b'=') else {
        return false;
    };
    if eq == 0 {
        return false;
    }
    b[..eq]
        .iter()
        .enumerate()
        .all(|(i, &c)| c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

/// Lexes `line` into simple commands, each a list of words with quotes
/// removed (separators `; & | ( )`, newline, backtick and `$(`; `#`
/// comments; backslash escapes and line continuations).
pub fn split_commands(line: &str) -> Vec<Vec<String>> {
    let b = line.as_bytes();
    let mut cmds: Vec<Vec<String>> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut in_word = false;
    fn end_word(words: &mut Vec<String>, cur: &mut Vec<u8>, in_word: &mut bool) {
        if *in_word {
            words.push(String::from_utf8_lossy(cur).into_owned());
            cur.clear();
            *in_word = false;
        }
    }
    fn end_cmd(
        cmds: &mut Vec<Vec<String>>,
        words: &mut Vec<String>,
        cur: &mut Vec<u8>,
        in_word: &mut bool,
    ) {
        end_word(words, cur, in_word);
        if !words.is_empty() {
            cmds.push(std::mem::take(words));
        }
    }
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'\'' => {
                in_word = true;
                match b[i + 1..].iter().position(|&x| x == b'\'') {
                    None => {
                        cur.extend_from_slice(&b[i + 1..]);
                        i = b.len();
                    }
                    Some(j) => {
                        cur.extend_from_slice(&b[i + 1..i + 1 + j]);
                        i += j + 1;
                    }
                }
            }
            b'"' => {
                in_word = true;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' && i + 1 < b.len() && b"\"\\$`\n".contains(&b[i + 1]) {
                        i += 1;
                        if b[i] == b'\n' {
                            i += 1;
                            continue;
                        }
                    }
                    cur.push(b[i]);
                    i += 1;
                }
            }
            b'\\' => {
                if i + 1 < b.len() {
                    i += 1;
                    if b[i] == b'\n' || (b[i] == b'\r' && i + 1 < b.len() && b[i + 1] == b'\n') {
                        if b[i] == b'\r' {
                            i += 1;
                        }
                    } else {
                        in_word = true;
                        cur.push(b[i]);
                    }
                }
            }
            b'#' if !in_word => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                end_cmd(&mut cmds, &mut words, &mut cur, &mut in_word);
            }
            b' ' | b'\t' | b'\r' => end_word(&mut words, &mut cur, &mut in_word),
            b';' | b'&' | b'|' | b'(' | b')' | b'\n' | b'`' => {
                end_cmd(&mut cmds, &mut words, &mut cur, &mut in_word)
            }
            b'$' if i + 1 < b.len() && b[i + 1] == b'(' => {
                end_cmd(&mut cmds, &mut words, &mut cur, &mut in_word);
                i += 1;
            }
            _ => {
                in_word = true;
                cur.push(c);
            }
        }
        i += 1;
    }
    end_cmd(&mut cmds, &mut words, &mut cur, &mut in_word);
    cmds
}

//! Ports of I:internal/scrub/skipfile_test.go and strict_test.go.

use scrub::{skip_file, strict, strict_dropped};

#[test]
fn skip_file_cases() {
    for (path, want) in [
        (".env", true),
        (".ENV", true),
        ("app/.env", true),
        (r"C:\repo\app\.env", true),
        (".env.local", true),
        ("web/.env.production", true),
        (".env.example", true),
        ("server.pem", true),
        (r"certs\Server.PEM", true),
        ("tls/private.key", true),
        ("store.p12", true),
        ("store.PFX", true),
        ("id_rsa", true),
        ("id_rsa.pub", true),
        ("~/.ssh/id_ed25519", true),
        (r"C:\Users\me\.ssh\id_ecdsa", true),
        ("credentials", true),
        ("credentials.json", true),
        ("aws/Credentials", true),
        (".netrc", true),
        ("home/.npmrc", true),
        (".PYPIRC", true),
        ("a/b\\c/.env", true),
        // not secrets
        ("env.go", false),
        ("dotenv.go", false),
        (".envrc", false),
        ("environment.ts", false),
        ("key.go", false),
        ("keys/README.md", false),
        ("my.pem.bak", false),
        ("pem/readme.txt", false),
        (".env/readme.md", false),
        ("tokens.go", false),
        ("rsa_id", false),
        ("my_credentials.go", false),
        (".npmrc.bak", false),
        (".netrc.d/x", false),
        ("", false),
        ("dir/", false),
        (r"dir\", false),
    ] {
        assert_eq!(skip_file(path), want, "SkipFile({path:?})");
    }
}

/// Win32 drops trailing dots and spaces, and an alternate data stream names
/// the same file.
#[test]
fn skip_file_windows_names() {
    for (path, want) in [
        ("key.pem.", true),
        ("key.pem. .", true),
        ("id_rsa ", true),
        (".env::$DATA", true),
        ("x.pem:Zone.Identifier", true),
        (r"C:\repo\.env:stream:$DATA", true),
        ("C:.env", true),
        ("C:key.pem", true),
        (r"C:\repo\notes.txt:secret.pem", false),
        ("..", false),
        ("C:", false),
        ("readme.md.", false),
    ] {
        assert_eq!(skip_file(path), want, "SkipFile({path:?})");
    }
}

/// Share to board's stricter pass: absolute paths and home directories go;
/// relative paths, URLs and prose stay.
#[test]
fn strict_drops_paths() {
    for (input, want) in [
        (
            r"fix C:\Users\pat\projects\irori\main.go now",
            "fix [path] now",
        ),
        ("see c:/work/x", "see [path]"),
        (r"under \\nas\share\docs", "under [path]"),
        ("in ~/projects/mlg", "in [path]"),
        ("in ~", "in [path]"),
        ("at $HOME/.config/x", "at [path]"),
        (r"at %USERPROFILE%\AppData\Local\irori", "at [path]"),
        ("/Users/pat/projects/mlg/x.go breaks", "[path] breaks"),
        (r#"log ("/home/me/logs/a.txt")"#, r#"log ("[path]")"#),
        ("path=/var/lib/irori/x", "path=[path]"),
        ("see `/Users/pat/projects/x`", "see `[path]`"),
        ("in `~/projects/x`", "in `[path]`"),
        ("run `C:\\Users\\me\\x.exe` first", "run `[path]` first"),
        ("file:///home/me/x.txt", "[path]"),
        ("open FILE://nas/share/x.txt now", "open [path] now"),
        ("at:/home/me/x", "at:[path]"),
        ("*/home/me/x*", "*[path]*"),
        ("(/Users/me/x)", "([path])"),
        ("[~/notes]", "[[path]]"),
        ("**/etc/irori/conf**", "**[path]**"),
        ("`\\\\nas\\share\\x`", "`[path]`"),
        // kept
        (
            "edit src/coord/impl.go and ./build/x",
            "edit src/coord/impl.go and ./build/x",
        ),
        (
            "https://github.com/StephenSHorton/irori/pull/12",
            "https://github.com/StephenSHorton/irori/pull/12",
        ),
        ("read and/or write 1/2 of it", "read and/or write 1/2 of it"),
        ("use /help", "use /help"),
        ("plain title", "plain title"),
    ] {
        assert_eq!(strict(input), want, "Strict({input:?})");
    }
    assert!(strict_dropped(r"C:\x") && !strict_dropped("nothing here"));
}

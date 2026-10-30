//! Ports of I:internal/coord/glob_test.go and links_test.go (tables
//! verbatim, both argument orders for Overlaps).
#![allow(clippy::type_complexity)]

use coord::*;

#[test]
fn test_overlaps() {
    let cases: &[(&str, &str, bool)] = &[
        ("a.go", "a.go", true),
        ("a.go", "b.go", false),
        ("src/a.go", "src/a.go", true),
        ("src/a.go", "lib/a.go", false),
        ("src/a.go", "src", false),
        ("src", "src/a.go", false),
        ("./src/a.go", "src/a.go", true),
        ("Src/A.go", "src/a.go", true),
        ("*", "a.go", true),
        ("*", "src/a.go", false),
        ("src/*", "src/a.go", true),
        ("src/*", "src/sub/a.go", false),
        ("*.go", "*.ts", false),
        ("*.go", "a.*", true),
        ("a*", "*b", true),
        ("a*b", "a*c", false),
        ("ab*", "a*", true),
        ("x*y*z", "*q*", true),
        ("*a*", "b", false),
        ("**.go", "x.go", true),
        ("a?.go", "ab.go", true),
        ("a?.go", "a.go", false),
        ("???", "ab", false),
        ("???", "*", true),
        ("?", "**", true),
        ("[abc].go", "b.go", true),
        ("[abc].go", "d.go", false),
        ("[a-c].go", "[b-d].go", true),
        ("[0-3]", "[4-9]", false),
        ("[0-3]", "[3-9]", true),
        ("[^0].go", "0.go", false),
        ("[^a].go", "a.go", true),
        ("[^a].go", "b.go", true),
        ("[^0-9]", "[0-9]", false),
        ("[^0-4]", "[0-9]", true),
        ("[^x]", "[^y]", true),
        ("[a-z]", "[0-9]", false),
        ("[a-c]", "[X-Z]", true),
        ("[A]", "a", true),
        ("?", "[0]", true),
        ("**", "anything/at/all.go", true),
        ("**", "x", true),
        ("src/**", "src/a/b/c.go", true),
        ("src/**", "src", true),
        ("src/**", "lib/a.go", false),
        ("src/", "src/a.go", true),
        ("**/a.go", "a.go", true),
        ("**/a.go", "x/y/a.go", true),
        ("**/a.go", "x/y/b.go", false),
        ("src/**/test/*.go", "src/test/a.go", true),
        ("src/**/test/*.go", "src/x/y/test/a.go", true),
        ("src/**/test/*.go", "src/x/y/test/sub/a.go", false),
        ("src/**/test/*.go", "lib/**", false),
        ("a/**/z", "a/b/**", true),
        ("a/**/z", "**/y", false),
        ("**/x/**", "**/y/**", true),
        ("a/**/b/**/c", "a/b/c", true),
        ("a/**", "**/b", true),
        ("*/b", "a/*", true),
        ("*/b", "a/*/c", false),
        ("a/*/c", "a/**", true),
        ("a/b", "a/b/c", false),
        ("a/*/*", "a/*", false),
        ("[", "a", true),
        (r"a\b", "x", true),
        ("/abs", "x", true),
    ];
    for (a, b, want) in cases {
        assert_eq!(overlaps(a, b), *want, "Overlaps({a:?}, {b:?})");
        assert_eq!(overlaps(b, a), *want, "Overlaps({b:?}, {a:?}) swapped");
    }
    // SimpleFold orbits, not ASCII case: K and the Kelvin sign, s and long s.
    assert!(overlaps("\u{212A}.go", "k.go"));
    assert!(overlaps("\u{17F}rc", "SRC"));
}

#[test]
fn test_parse_resource() {
    let ok = [
        (
            "path:remote:github.com/a/b:src/**",
            "path:remote:github.com/a/b:src/**",
        ),
        (
            "path:root:4b825dc642cb6eb9a060e54bf8d69288fbee4904:src/*.go",
            "path:root:4b825dc642cb6eb9a060e54bf8d69288fbee4904:src/*.go",
        ),
        (
            "path:remote:github.com/a/b:./docs/",
            "path:remote:github.com/a/b:docs/**",
        ),
        ("path:root:abc:ab?c.txt", "path:root:abc:ab?c.txt"),
        ("path:remote:::1/a/b:src", "path:remote:::1/a/b:src"),
        (
            "path:remote:host.com/a:b/c:src/",
            "path:remote:host.com/a:b/c:src/**",
        ),
        (
            "path:remote:host.com/my org/repo:x",
            "path:remote:host.com/my org/repo:x",
        ),
        (
            "path:remote:github.com/a/b:**",
            "path:remote:github.com/a/b:**",
        ),
        ("  gpu/qwen ", "gpu/qwen"),
        ("build-cache", "build-cache"),
        ("db/main.v2", "db/main.v2"),
        ("port:8799", "port:8799"),
        ("port:1", "port:1"),
        ("port:65535", "port:65535"),
    ];
    for (input, want) in ok {
        assert_eq!(normalize_resource(input).as_deref(), Ok(want), "{input:?}");
    }
    let r = parse_resource("path:root:abc:ab?c.txt").unwrap();
    assert_eq!(
        (r.kind, r.repo_key.as_str(), r.glob.as_str()),
        (RESOURCE_PATH, "root:abc", "ab?c.txt")
    );
    let r = parse_resource("path:remote:github.com/a/b:src/**").unwrap();
    assert_eq!(
        (r.repo_key.as_str(), r.glob.as_str()),
        ("remote:github.com/a/b", "src/**")
    );
    let bad = [
        "",
        "path:",
        "path:remote:github.com/a/b",
        "path:remote:github.com/a/b:",
        "path:remote::src/**",
        "path:other:x:src",
        "path:github.com/a/b:src",
        r"path:remote:github.com/a/b:src\a.go",
        "path:remote:github.com/a/b:/etc/passwd",
        "path:root:abc:C:/Windows/x",
        "path:root:abc:./C:/x",
        "path:root:abc:.//etc/x",
        "path:root:abc:c:x",
        "path:root:abc:ab:c.txt",
        "path:root:ABC:x",
        "path:root:xyz:x",
        "path:remote: host/a/b:x",
        "path:remote:host/a\x01b:x",
        "path:root:abc:a\nb",
        "path:root:abc:src/\x00x",
        "path:root:abc:../x",
        "path:root:abc:a/../x",
        "path:root:abc:a/./x",
        "path:root:abc:a//x",
        "path:root:abc:./",
        "path:root:abc:[",
        "path:root:abc:a/[/]",
        "port:0",
        "port:65536",
        "port:080",
        "port:x",
        "port:",
        "GPU/qwen",
        "gpu/",
        "/gpu",
        "-gpu",
        "gpu//qwen",
        "role:engine",
        "gpu qwen",
    ];
    for input in bad {
        let r = normalize_resource(input);
        assert!(
            r.as_ref().is_err_and(|e| e.is(ErrorKind::BadResource)),
            "{input:?}: {r:?}"
        );
    }
    // Go's texts.
    assert_eq!(
        normalize_resource("gpu qwen").unwrap_err().to_string(),
        "coord: resource must be path:<repoKey>:<glob> or a name like gpu/qwen (\"gpu qwen\")"
    );
    assert_eq!(
        normalize_resource("port:080").unwrap_err().to_string(),
        "coord: resource must be path:<repoKey>:<glob> or a name like gpu/qwen: port must be 1-65535 (\"port:080\")"
    );
}

#[test]
fn test_parse_link() {
    struct C {
        input: &'static str,
        kind: &'static str,
        reference: &'static str,
        label: &'static str,
        bad: bool,
    }
    let c = |input, kind, label| C {
        input,
        kind,
        reference: "",
        label,
        bad: false,
    };
    let r = |input, kind, reference, label| C {
        input,
        kind,
        reference,
        label,
        bad: false,
    };
    let b = |input| C {
        input,
        kind: "",
        reference: "",
        label: "",
        bad: true,
    };
    let cases = [
        c(
            "https://linear.app/acme/issue/abc-123/fix-the-thing",
            LINK_URL,
            "Linear ABC-123",
        ),
        c("https://linear.app/acme/project/x", LINK_URL, "Linear"),
        c(
            "https://github.com/StephenSHorton/irori/issues/57",
            LINK_URL,
            "GitHub #57",
        ),
        c(
            "https://github.com/StephenSHorton/irori/pull/12/files",
            LINK_URL,
            "GitHub #12",
        ),
        c(
            "https://github.com/StephenSHorton/irori",
            LINK_URL,
            "GitHub",
        ),
        c(
            "https://www.notion.so/acme/Board-design-0123456789abcdef0123456789abcdef",
            LINK_URL,
            "Notion Board design",
        ),
        c(
            "https://www.notion.so/0123456789abcdef0123456789abcdef",
            LINK_URL,
            "Notion",
        ),
        c(
            "https://acme.notion.site/Plan-0123456789abcdef0123456789abcdef",
            LINK_URL,
            "Notion Plan",
        ),
        c("https://www.example.com/a/b", LINK_URL, "example.com"),
        r("ABCDEF1234", LINK_COMMIT, "abcdef1234", "commit abcdef1"),
        r(
            r"docs\design\board.md",
            LINK_PATH,
            "docs/design/board.md",
            ".../design/board.md",
        ),
        c("README.md", LINK_PATH, "README.md"),
        c("C:/work/notes.txt", LINK_PATH, ".../work/notes.txt"),
        c("board.go:580", LINK_PATH, "board.go:580"),
        c(
            "internal/coord/cards_impl.go:347",
            LINK_PATH,
            ".../coord/cards_impl.go:347",
        ),
        b("ftp://example.com/x"),
        b("javascript:alert(1)"),
        b("mailto:x@example.com"),
        b("data:text/html,<b>x</b>"),
        b("https:example.com"),
        b("https://github.com@evil.com/x"),
        b("https://user:pw@example.com/"),
        b("https://"),
        b("../secrets"),
        b(""),
        b("a\nb"),
    ];
    for tc in cases {
        let got = parse_link(tc.input);
        if tc.bad {
            assert!(
                got.as_ref().is_err_and(|e| e.is(ErrorKind::Invalid)),
                "{:?}: {got:?}",
                tc.input
            );
            continue;
        }
        let want_ref = if tc.reference.is_empty() {
            tc.input
        } else {
            tc.reference
        };
        let l = got.unwrap_or_else(|e| panic!("{:?}: {e}", tc.input));
        assert_eq!(
            (l.kind.as_str(), l.reference.as_str(), l.label.as_str()),
            (tc.kind, want_ref, tc.label),
            "{:?}",
            tc.input
        );
    }
}

#[test]
fn test_pr_link() {
    let l = parse_link("https://github.com/a/b/pull/12/files").unwrap();
    assert_eq!(pr_link(&l), Some(("a/b".to_string(), 12)));
    let l = parse_link("https://www.GitHub.com/a.b/c-d/pull/7").unwrap();
    assert_eq!(pr_link(&l), Some(("a.b/c-d".to_string(), 7)));
    for s in [
        "https://github.com/a/b/issues/12",
        "https://gitlab.com/a/b/pull/12",
        "src/pull/3",
        "https://github.com/a/b/pull/0",
    ] {
        let l = parse_link(s).unwrap();
        assert_eq!(pr_link(&l), None, "{s}");
    }
    assert_eq!(link_label("https://github.com/a/b/pull/3"), "GitHub #3");
}

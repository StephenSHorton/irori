//! Ports of I:internal/scrub/scrub_test.go and regress_test.go.
//!
//! Fake tokens are built by concatenation so the repo never holds a string
//! a secret scanner would flag. None of these is a real credential.

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use scrub::{Scrubber, contains_any_fold};

fn rep(s: &str, n: usize) -> String {
    s.repeat(n)
}

struct Fakes {
    anthropic: String,
    openai: String,
    open_proj: String,
    ghp: String,
    gho: String,
    ghs: String,
    ghu: String,
    gh_pat: String,
    slack_b: String,
    slack_p: String,
    aws: String,
    aws_temp: String,
    google: String,
    jwt: String,
    bearer: String,
}

fn fakes() -> Fakes {
    Fakes {
        anthropic: format!("sk-ant-api03-{}", rep("a1B2", 10)),
        openai: format!("sk-{}", rep("Ab3d", 12)),
        open_proj: format!("sk-proj-{}", rep("Xy9_", 20)),
        ghp: format!("gh{}{}", "p_", rep("A1b2C3", 6)),
        gho: format!("gh{}{}", "o_", rep("Z9y8X7", 6)),
        ghs: format!("gh{}{}", "s_", rep("q1W2e3", 6)),
        ghu: format!("gh{}{}", "u_", rep("r4T5y6", 6)),
        gh_pat: format!("github_pat_{}_{}", rep("11AB", 6), rep("cd34", 14)),
        slack_b: format!("xo{}1234567890-1234567890123-{}", "xb-", rep("aB3", 8)),
        slack_p: format!("xo{}1234567890-{}", "xp-", rep("Zz9", 6)),
        aws: format!("AK{}IOSFODNN7FAKE012", "IA"),
        aws_temp: format!("AS{}{}", "IA", rep("Q7", 8)),
        google: format!("AI{}Sy{}abc", "za", rep("D-x_9", 6)),
        jwt: format!(
            "ey{}.ey{}.{}",
            "JhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9",
            "JzdWIiOiIxMjM0NTY3ODkwIn0",
            rep("Sf1Kx", 8)
        ),
        bearer: format!("abc123{}", rep("XyZ.-_", 5)),
    }
}

fn fake_pem(kind: &str, nl: &str, body_lines: usize, prefix: &str) -> String {
    let mut b = format!("{prefix}-----BEGIN {kind}PRIVATE KEY-----{nl}");
    for i in 0..body_lines {
        b.push_str(&format!("{prefix}{}{nl}", rep(&format!("M{i}IEvQ"), 10)));
    }
    b.push_str(&format!("{prefix}-----END {kind}PRIVATE KEY-----"));
    b
}

fn lines(s: &str) -> (usize, usize) {
    (s.matches('\n').count(), s.matches('\r').count())
}

#[track_caller]
fn check(s: &Scrubber, input: &str, want: &str) {
    let got = s.string(input);
    assert_eq!(got, want, "String({input:?})");
    assert_eq!(lines(&got), lines(input), "String({input:?}): line breaks");
    assert_eq!(s.string(&got), got, "not idempotent: {got:?}");
}

#[test]
fn token_shapes() {
    let f = fakes();
    let s = Scrubber::default();
    let cases: Vec<(String, &str)> = vec![
        (f.anthropic.clone(), "[redacted:anthropic]"),
        (
            format!("key {} end", f.anthropic),
            "key [redacted:anthropic] end",
        ),
        (f.openai.clone(), "[redacted:openai]"),
        (f.open_proj.clone(), "[redacted:openai]"),
        (format!("use {}.", f.openai), "use [redacted:openai]."),
        (f.ghp.clone(), "[redacted:github]"),
        (f.gho.clone(), "[redacted:github]"),
        (f.ghs.clone(), "[redacted:github]"),
        (f.ghu.clone(), "[redacted:github]"),
        (f.gh_pat.clone(), "[redacted:github]"),
        (
            format!("https://{}@github.com/o/r.git", f.ghp),
            "https://[redacted:github]@github.com/o/r.git",
        ),
        (f.slack_b.clone(), "[redacted:slack]"),
        (f.slack_p.clone(), "[redacted:slack]"),
        (f.aws.clone(), "[redacted:aws]"),
        (format!("id={};", f.aws_temp), "id=[redacted:aws];"),
        (f.google.clone(), "[redacted:google]"),
        (
            format!("?key={}&x=1", f.google),
            "?key=[redacted:google]&x=1",
        ),
        (f.jwt.clone(), "[redacted:jwt]"),
        (
            format!("cookie: s={}; Path=/", f.jwt),
            "cookie: s=[redacted:jwt]; Path=/",
        ),
        (
            format!("a {} b {} c", f.ghp, f.slack_b),
            "a [redacted:github] b [redacted:slack] c",
        ),
        (format!("\"{}\"", f.anthropic), "\"[redacted:anthropic]\""),
    ];
    for (i, w) in cases {
        check(&s, &i, w);
    }
}

#[test]
fn private_key() {
    let s = Scrubber::default();
    let cases: Vec<(&str, String, &str)> = vec![
        ("rsa lf", fake_pem("RSA ", "\n", 3, ""), "[redacted:private-key]\n\n\n\n"),
        (
            "plain crlf",
            fake_pem("", "\r\n", 2, ""),
            "[redacted:private-key]\r\n\r\n\r\n",
        ),
        (
            "ec in text",
            format!("before\n{}\nafter", fake_pem("EC ", "\n", 1, "")),
            "before\n[redacted:private-key]\n\n\nafter",
        ),
        (
            "openssh",
            fake_pem("OPENSSH ", "\n", 2, ""),
            "[redacted:private-key]\n\n\n",
        ),
        (
            "patch lines keep +",
            format!("@@ -0,0 +1,4 @@\n{}\n context", fake_pem("RSA ", "\n", 2, "+")),
            "@@ -0,0 +1,4 @@\n+[redacted:private-key]\n+\n+\n+\n context",
        ),
        (
            "patch removal lines keep -",
            fake_pem("", "\n", 1, "-"),
            "-[redacted:private-key]\n-\n-",
        ),
        (
            "indented yaml",
            format!("key: |\n{}\nnext: 1", fake_pem("", "\n", 1, "  ")),
            "key: |\n  [redacted:private-key]\n  \n  \nnext: 1",
        ),
        (
            "json escaped one line",
            r#"{"private_key":"-----BEGIN PRIVATE KEY-----\nMIIEvQIBADAN\n-----END PRIVATE KEY-----\n"}"#
                .to_string(),
            r#"{"private_key":"[redacted:private-key]\n"}"#,
        ),
        (
            "truncated, no END",
            "x\n-----BEGIN RSA PRIVATE KEY-----\nMIIEow\nMIIEow".to_string(),
            "x\n[redacted:private-key]\n\n",
        ),
        (
            "two blocks",
            format!(
                "{}\nmid\n{}",
                fake_pem("", "\n", 1, ""),
                fake_pem("", "\n", 1, "")
            ),
            "[redacted:private-key]\n\n\nmid\n[redacted:private-key]\n\n",
        ),
        (
            "public key untouched",
            "-----BEGIN PUBLIC KEY-----\nMIIBIjAN\n-----END PUBLIC KEY-----".to_string(),
            "-----BEGIN PUBLIC KEY-----\nMIIBIjAN\n-----END PUBLIC KEY-----",
        ),
        (
            "certificate untouched",
            "-----BEGIN CERTIFICATE-----\nMIIBIjAN\n-----END CERTIFICATE-----".to_string(),
            "-----BEGIN CERTIFICATE-----\nMIIBIjAN\n-----END CERTIFICATE-----",
        ),
        (
            "prose",
            "never paste your PRIVATE KEY here".to_string(),
            "never paste your PRIVATE KEY here",
        ),
    ];
    for (name, i, w) in cases {
        let got = s.string(&i);
        assert_eq!(got, w, "{name}");
        check(&s, &i, w);
    }
}

#[test]
fn bearer() {
    let f = fakes();
    let s = Scrubber::default();
    let b64 = base64::engine::general_purpose::STANDARD.encode("user:password");
    let cases: Vec<(String, String)> = vec![
        (
            format!("Authorization: Bearer {}", f.bearer),
            "Authorization: Bearer [redacted:bearer]".into(),
        ),
        (
            format!("authorization: bearer {}", f.bearer),
            "authorization: bearer [redacted:bearer]".into(),
        ),
        (
            format!("curl -H \"Authorization: Bearer {}\" https://x", f.bearer),
            "curl -H \"Authorization: Bearer [redacted:bearer]\" https://x".into(),
        ),
        (
            format!("{{\"Authorization\": \"Bearer {}\"}}", f.bearer),
            "{\"Authorization\": \"Bearer [redacted:bearer]\"}".into(),
        ),
        (
            format!("Bearer\t{}==", f.bearer),
            "Bearer\t[redacted:bearer]".into(),
        ),
        (
            format!("Authorization: Bearer {}", f.jwt),
            "Authorization: Bearer [redacted:jwt]".into(),
        ),
        (
            format!("Authorization: Basic {b64}"),
            "Authorization: Basic [redacted:bearer]".into(),
        ),
        (
            format!("Authorization: token {}", f.bearer),
            "Authorization: token [redacted:bearer]".into(),
        ),
        (
            "Authorization: Bearer $TOKEN".into(),
            "Authorization: Bearer $TOKEN".into(),
        ),
        (
            "Authorization: Bearer ${TOKEN}".into(),
            "Authorization: Bearer ${TOKEN}".into(),
        ),
        ("Bearer abc".into(), "Bearer abc".into()),
        (
            "the bearer instruments rose".into(),
            "the bearer instruments rose".into(),
        ),
        // never across a line break
        (
            "Bearer\nabcdef123456789".into(),
            "Bearer\nabcdef123456789".into(),
        ),
    ];
    for (i, w) in cases {
        check(&s, &i, &w);
    }
}

#[test]
fn assignment() {
    let f = fakes();
    let s = Scrubber::default();
    let v = "hunter2hunter2";
    let a = "[redacted:assignment]";
    let cases: Vec<(String, String)> = vec![
        (format!("password={v}"), format!("password={a}")),
        (format!("password = {v}"), format!("password = {a}")),
        (format!("password: {v}"), format!("password: {a}")),
        (format!("password: \"{v}\""), format!("password: \"{a}\"")),
        (format!("password = '{v}'"), format!("password = '{a}'")),
        (format!("token := `{v}`"), format!("token := `{a}`")),
        (
            format!("\"password\": \"{v}\""),
            format!("\"password\": \"{a}\""),
        ),
        (
            format!("{{\"password\":\"{v}\",\"user\":\"bob\"}}"),
            format!("{{\"password\":\"{a}\",\"user\":\"bob\"}}"),
        ),
        (
            format!("{{\\\"password\\\":\\\"{v}\\\"}}"),
            format!("{{\\\"password\\\":\\\"{a}\\\"}}"),
        ),
        (
            format!("'client_secret' => '{v}',"),
            format!("'client_secret' => '{a}',"),
        ),
        (format!("DB_PASSWORD={v}"), format!("DB_PASSWORD={a}")),
        (
            format!("export API_TOKEN={v}"),
            format!("export API_TOKEN={a}"),
        ),
        (
            format!("export API_KEY=\"{v}\""),
            format!("export API_KEY=\"{a}\""),
        ),
        (format!("set APIKEY={v}"), format!("set APIKEY={a}")),
        (
            format!("$env:MY_SECRET = '{v}'"),
            format!("$env:MY_SECRET = '{a}'"),
        ),
        (format!("passwd: {v}"), format!("passwd: {a}")),
        (format!("api-key: {v}"), format!("api-key: {a}")),
        (
            format!("aws_secret_access_key = {}", rep("fAkE7", 8)),
            format!("aws_secret_access_key = {a}"),
        ),
        (
            format!("x-access-token: {v}"),
            format!("x-access-token: {a}"),
        ),
        (
            format!("https://h/cb?access_token={v}&state=1"),
            format!("https://h/cb?access_token={a}&state=1"),
        ),
        (format!("--password={v}"), format!("--password={a}")),
        ("password=12345678".into(), format!("password={a}")),
        (format!("Password: {v}"), format!("Password: {a}")),
        ("SECRET=\"a b c d e f\"".into(), format!("SECRET=\"{a}\"")),
        (
            format!("token = \"{}\"", f.ghp),
            "token = \"[redacted:github]\"".into(),
        ),
        (
            format!("a=1\r\nTOKEN={v}\r\nb=2"),
            format!("a=1\r\nTOKEN={a}\r\nb=2"),
        ),
        (
            format!("token={v} secret={v}"),
            format!("token={a} secret={a}"),
        ),
        (
            "password: 'abc\\'defghij'".into(),
            format!("password: '{a}'"),
        ),
        (
            "\"secret\": \"abc\\\"defghij\"".into(),
            format!("\"secret\": \"{a}\""),
        ),
        (
            "https://[redacted:github]:pw12345678@h".into(),
            format!("https://[redacted:github]:{a}@h"),
        ),
        (
            format!("postgres://app:{v}@db:5432/x"),
            format!("postgres://app:{a}@db:5432/x"),
        ),
        (
            "https://user:pw@example.com/".into(),
            format!("https://user:{a}@example.com/"),
        ),
        (
            format!("redis://default:{v}@cache"),
            format!("redis://default:{a}@cache"),
        ),
    ];
    for (i, w) in cases {
        check(&s, &i, &w);
    }
}

#[test]
fn env_file() {
    let f = fakes();
    let s = Scrubber::default();
    let input = format!(
        "# app settings\r\nPORT=8080\r\nDATABASE_URL=postgres://app:s3cretpass@localhost/app\r\nSTRIPE_SECRET_KEY=rk_{}\r\nGITHUB_TOKEN={}\r\nSESSION_SECRET=\r\nDEBUG=true\r\n",
        rep("live9", 6),
        f.ghp
    );
    let want = "# app settings\r\nPORT=8080\r\nDATABASE_URL=postgres://app:[redacted:assignment]@localhost/app\r\nSTRIPE_SECRET_KEY=[redacted:assignment]\r\nGITHUB_TOKEN=[redacted:github]\r\nSESSION_SECRET=\r\nDEBUG=true\r\n";
    check(&s, &input, want);
}

/// Ordinary code and prose must come through unchanged.
#[test]
fn false_positives() {
    let s = Scrubber::default();
    let plain_b64 = base64::engine::general_purpose::STANDARD
        .encode("the quick brown fox jumps over the lazy dog, twice over");
    let json_b64 =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"name":"irori","kind":"hub"}"#);
    let mut cases: Vec<String> = [
        "tokenCount++",
        "const maxTokens = 4096",
        "maxTokens: number;",
        "let tokenCount = 0",
        "password: string;",
        "  password: string | undefined,",
        "interface Login { password: string }",
        "apiKey?: string",
        "secret: bool",
        "password: Optional[str] = None",
        "const token = getToken();",
        "token = os.Getenv(\"GITHUB_TOKEN\")",
        "apiKey: process.env.OPENAI_API_KEY,",
        "token: config.auth.token",
        "TOKEN=$GITHUB_TOKEN",
        "set TOKEN=%GITHUB_TOKEN%",
        "password = ${DB_PASS}",
        "token := cfg[\"token\"]",
        "if token == \"\" {",
        "func (s *Store) Token() string {",
        "secretsManager.getSecretValue({ SecretId: id })",
        "The secret to good code is small functions.",
        "Keep your password safe; never share it.",
        "Use sk- prefixed keys from the dashboard.",
        "sk-learn is a typo for scikit-learn",
        "task-abcdefghijklmnopqrstuvwxyz0123",
        "sk-some-long-hyphenated-words-in-prose",
        "password=short",
        "token: abc",
        "\"password\": \"hunter\"",
        "AKIA is the prefix for access key ids",
        "AIza prefix",
        "eyJ is how base64 JSON starts",
        "ghp_short",
        "github_pat_",
        "xoxb-123",
        "https://example.com:8443/path",
        "git@github.com:org/repo.git",
        "mailto:user@example.com",
        "Bearer",
        "tokenizer.encode(text)",
        "-----BEGIN PUBLIC KEY-----",
        "",
        "\n\n\r\n",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    cases.push(json_b64);
    cases.push(plain_b64);
    for i in cases {
        check(&s, &i, &i);
    }
}

#[test]
fn exact() {
    let f = fakes();
    let lock = format!("irori-lock-{}", rep("7f", 12));
    let attach = format!("attach-{}", rep("Q", 10));
    let s = Scrubber::new(&[lock.as_str(), "", "short", attach.as_str()]);
    check(
        &s,
        &format!("token in file: {lock}\nsee {attach}"),
        "token in file: [redacted:exact]\nsee [redacted:exact]",
    );
    check(&s, "short stays", "short stays");
    check(
        &s,
        &format!("{lock}{lock}"),
        "[redacted:exact][redacted:exact]",
    );

    // Exact wins over shape, so Irori's own value is tagged exact.
    s.set_exact(&[f.ghp.as_str()]);
    check(&s, &format!("x {}", f.ghp), "x [redacted:exact]");

    // set_exact replaces the list.
    s.set_exact(&[attach.as_str()]);
    check(&s, &lock, &lock);
    check(&s, &attach, "[redacted:exact]");

    // Longest value wins when one contains another.
    s.set_exact(&["abcdefgh", "abcdefghijkl"]);
    check(
        &s,
        "abcdefghijkl abcdefgh",
        "[redacted:exact] [redacted:exact]",
    );

    // A multi-line exact value keeps its line breaks.
    s.set_exact(&["line-one-value\r\nline-two-value"]);
    check(
        &s,
        "a line-one-value\r\nline-two-value b",
        "a [redacted:exact]\r\n b",
    );

    s.set_exact::<&str>(&[]);
    check(&s, &attach, &attach);

    check(&Scrubber::default(), "nothing here", "nothing here");
}

#[test]
fn concurrent() {
    let s = Arc::new(Scrubber::new(&["value-zero-0000"]));
    let mut hs = Vec::new();
    for g in 0..8 {
        let w = s.clone();
        hs.push(std::thread::spawn(move || {
            for i in 0..200 {
                let v = format!("value-{g}-{i:04}");
                w.set_exact(&[v.as_str(), "value-zero-0000"]);
            }
        }));
        let r = s.clone();
        hs.push(std::thread::spawn(move || {
            for _ in 0..200 {
                assert_eq!(r.string("x value-zero-0000 y"), "x [redacted:exact] y");
            }
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
}

fn sample_patch() -> String {
    let f = fakes();
    format!(
        "--- a/config/app.go\n+++ b/config/app.go\n@@ -1,6 +1,14 @@\n package config\n-const maxTokens = 2048\n+const maxTokens = 4096\n+var anthropicKey = \"{}\"\n+var gh = \"{}\" // {}\n+// Authorization: Bearer {}\n+password := \"hunter2hunter2\"\n{}\n func tokenCount() int {{ return 0 }}\n",
        f.anthropic,
        f.ghp,
        f.aws,
        f.jwt,
        fake_pem("RSA ", "\n", 3, "+")
    )
}

/// Every rule on a mixed, multi-line patch keeps the line count and never
/// grows a line by more than one marker per secret.
#[test]
fn line_count() {
    let f = fakes();
    let s = Scrubber::default();
    let p = sample_patch();
    let out = s.string(&p);
    let a: Vec<&str> = p.split('\n').collect();
    let b: Vec<&str> = out.split('\n').collect();
    assert_eq!(a.len(), b.len());
    assert_ne!(out, p, "sample patch had nothing redacted");
    for i in 0..a.len() {
        assert!(
            b[i].len() <= a[i].len() + "[redacted:private-key]".len(),
            "line {i} grew: {:?} -> {:?}",
            a[i],
            b[i]
        );
    }
    for leak in [
        f.anthropic.as_str(),
        f.ghp.as_str(),
        f.aws.as_str(),
        f.jwt.as_str(),
        "hunter2hunter2",
        "M0IEvQ",
    ] {
        assert!(!out.contains(leak), "leaked {leak:?}");
    }
}

fn big_patch() -> String {
    let chunk = sample_patch();
    let plain = rep(
        " func (s *Store) Token() string { return s.tokenCount }\n",
        20,
    );
    let mut b = String::new();
    while b.len() < 4 << 20 {
        b.push_str(&chunk);
        b.push_str(&plain);
    }
    b
}

/// Go's `slow` (10 only under -race). The workspace builds `regex` and
/// `aho-corasick` optimized in dev, so Go's bound holds in debug tests.
fn slow() -> u32 {
    1
}

#[test]
fn big_patch_time() {
    let f = fakes();
    let s = Scrubber::new(&[format!("irori-lock-{}", rep("ab", 12))]);
    let p = big_patch();
    let start = Instant::now();
    let out = s.string(&p);
    let d = start.elapsed();
    assert!(
        d <= Duration::from_secs(10) * slow(),
        "4 MiB patch took {d:?}"
    );
    assert_eq!(out.matches('\n').count(), p.matches('\n').count());
    assert!(!out.contains(&f.ghp), "leaked");
}

/// One huge line (minified JSON) has no line splits to help.
#[test]
fn long_line_time() {
    let f = fakes();
    let s = Scrubber::default();
    let mut b = String::new();
    while b.len() < 1 << 20 {
        b.push_str(&format!(
            "{{\"tokenCount\":12,\"password\":\"hunter2hunter2\",\"note\":\"Bearer abc\",\"k\":\"{}\"}},",
            f.ghp
        ));
    }
    let start = Instant::now();
    let out = s.string(&b);
    let d = start.elapsed();
    assert!(
        d <= Duration::from_secs(10) * slow(),
        "1 MiB line took {d:?}"
    );
    assert!(
        !out.contains(&f.ghp) && !out.contains("hunter2hunter2"),
        "leaked"
    );
}

#[test]
fn contains_any_fold_cases() {
    let words = ["token", "api_key"];
    for (s, want) in [
        ("TOKEN", true),
        ("myToken=1", true),
        ("API_KEY", true),
        ("apiKey", false),
        ("tok", false),
        ("toke", false),
        ("", false),
        ("API\x7fKEY", false),
    ] {
        assert_eq!(contains_any_fold(s, &words), want, "containsAnyFold({s:?})");
    }
}

/// regress_test.go TestReviewFindings: each input comes back with the
/// secret redacted.
#[test]
fn review_findings() {
    let s = Scrubber::default();
    let discord = format!(
        "{}.{}.{}",
        "MTk4NjIyNDgzNDcxOTI1MjQ4", "Cl2FMQ", "ZnCjm1XVW7vRze4b7Cq4se7kKWs"
    );
    let a = "[redacted:assignment]";
    let cases: Vec<(&str, String, String)> = vec![
        (
            "apostrophe in double quotes",
            r#""password": "it's-a-secret-value-123""#.into(),
            format!("\"password\": \"{a}\""),
        ),
        (
            "double quote in single quotes",
            r#"token = 'abc"defghijklmnop'"#.into(),
            format!("token = '{a}'"),
        ),
        (
            "backtick in double quotes",
            "secret = \"abc`defghijkl\"".into(),
            format!("secret = \"{a}\""),
        ),
        (
            "quote in escaped json",
            r#"{\"password\":\"it's-a-secret-1\"}"#.into(),
            format!("{{\\\"password\\\":\\\"{a}\\\"}}"),
        ),
        (
            "discord env",
            format!("DISCORD_TOKEN={discord}"),
            format!("DISCORD_TOKEN={a}"),
        ),
        (
            "dotted letters",
            "token: abcdefgh.ijklmnop.qrstuvwx".into(),
            format!("token: {a}"),
        ),
        (
            "ampersand",
            "DB_PASSWORD=p&ssw0rd!longvalue".into(),
            format!("DB_PASSWORD={a}"),
        ),
        (
            "semicolon",
            "DB_PASSWORD=abc;def1234567890".into(),
            format!("DB_PASSWORD={a}"),
        ),
        (
            "comma",
            "api_key=abc,defghijklmnopq".into(),
            format!("api_key={a}"),
        ),
        (
            "pipe",
            "export TOKEN=abc|defghijklmnop".into(),
            format!("export TOKEN={a}"),
        ),
        (
            "parens",
            "SECRET_KEY=django-insecure-(abc)defghijk".into(),
            format!("SECRET_KEY={a}"),
        ),
        (
            "backslash",
            r"password=abc\defghijkl".into(),
            format!("password={a}"),
        ),
        (
            "trailing punctuation kept",
            "f(token=abcdefghijkl);".into(),
            format!("f(token={a});"),
        ),
        (
            "query param cut",
            "?token=abc&defghijkl&next=1".into(),
            format!("?token={a}&next=1"),
        ),
        (
            "escaped quote end",
            r#""cmd":"export TOKEN=abcdefghijkl\" ok""#.into(),
            format!("\"cmd\":\"export TOKEN={a}\\\" ok\""),
        ),
        (
            "base64 padding",
            "secret=QUJDREVGR0hJSg==".into(),
            format!("secret={a}"),
        ),
        (
            "object literal",
            "{ password: abcd1234efgh }".into(),
            format!("{{ password: {a} }}"),
        ),
        (
            "dollar",
            "PASSWORD=$ecretPassw0rd99".into(),
            format!("PASSWORD={a}"),
        ),
        (
            "at",
            "password: @dm1nPassw0rd!".into(),
            format!("password: {a}"),
        ),
        (
            "star",
            "password: *Hunter2Hunter2".into(),
            format!("password: {a}"),
        ),
        (
            "percent",
            "password=%Passw0rd123".into(),
            format!("password={a}"),
        ),
        (
            "url at in password",
            "https://user:p@ss0rdlong@host/x".into(),
            format!("https://user:{a}@host/x"),
        ),
        (
            "double equals",
            r#"if password == "hunter2hunter2" {"#.into(),
            format!("if password == \"{a}\" {{"),
        ),
        (
            "triple equals",
            "if (token === 'hunter2hunter2') {".into(),
            format!("if (token === '{a}') {{"),
        ),
        (
            "not equals",
            r#"if secret != "hunter2hunter2" {"#.into(),
            format!("if secret != \"{a}\" {{"),
        ),
    ];
    for (name, i, w) in cases {
        assert_eq!(s.string(&i), w, "{name}");
        check(&s, &i, &w);
    }
}

/// regress_test.go TestReviewFalsePositives.
#[test]
fn review_false_positives() {
    let s = Scrubber::default();
    for i in [
        "if token == expectedToken {",
        "if (token === undefined) {",
        "token = undefined",
        "password != nil",
        "token = *opts.Token",
        "token := *tokenPtr",
        "$token = $this->token;",
        "$password = $config['password'];",
        "TOKEN=$DISCORD_BOT_TOKEN",
        "PASSWORD=${DB_PASS:-default}",
        "TOKEN=$(cat /run/secrets/token)",
        "SECRET=$(security find-generic-password -w)",
        "password = hash(input)",
        "apiKey: Record<string, string>",
        "token: Map<string, number>;",
        "settings.SECRET_KEY = settings.SECRET_KEY",
        "secret: import.meta.env.VITE_SECRET,",
        "token: os.environ.get(\"TOKEN\")",
    ] {
        check(&s, i, i);
    }
}

#[test]
fn zero_scrubber() {
    let f = fakes();
    let z = Scrubber::default();
    assert_eq!(
        z.string("password=hunter2hunter2"),
        "password=[redacted:assignment]"
    );
    // A nil Go *Scrubber.
    assert_eq!(
        scrub::string(&format!("x {}", f.ghp)),
        "x [redacted:github]"
    );
    z.set_exact(&["my-exact-value-1"]);
    assert_eq!(z.string("a my-exact-value-1"), "a [redacted:exact]");
}

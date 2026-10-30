// Golden exporter for crates/scrub (capture.md §3 "scrub", §7.3-7.4, §7.7).
//
// This file is not part of the Go repo. tools/goldens/export.sh injects it
// into I:internal/scrub with `go test -overlay` (package scrub, so it reaches
// the rules, the test fakes and samplePatch). It only runs when
// IRORI_GOLDEN_OUT is set, and writes $IRORI_GOLDEN_OUT/scrub/*.json:
//
//	patterns.json  every compiled pattern's String(), by Go variable name
//	cases.json     String() over the test inputs plus adversarial ones
//	               (non-ASCII next to tokens, \v, Unicode folds in gates)
//	skipfile.json  SkipFile over a path corpus
//	strict.json    Strict over a text corpus
//
// Every token in here is a fake built by concatenation, as in scrub_test.go.
package scrub

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func goldScrubWrite(t *testing.T, rel string, v any) {
	t.Helper()
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	path := filepath.Join(dir, "scrub", rel)
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(v); err != nil {
		t.Fatal(err)
	}
	// The fake Slack tokens are written as xo\u0078… (the same JSON string)
	// so GitHub push protection doesn't take them for real ones; Go's own
	// test splits them ("xo" + "xb-") for the same reason.
	out := bytes.ReplaceAll(buf.Bytes(), []byte("xox"), []byte(`xo\u0078`))
	if err := os.WriteFile(path, out, 0o644); err != nil {
		t.Fatal(err)
	}
}

type goldPattern struct {
	Name    string `json:"name"`
	Pattern string `json:"pattern"`
}

func TestGoldenScrubPatterns(t *testing.T) {
	var out []goldPattern
	for i, r := range rules {
		out = append(out, goldPattern{fmt.Sprintf("rules[%d]", i), r.re.String()})
	}
	for _, p := range []struct {
		name string
		s    string
	}{
		{"assignmentRE", assignmentRE.String()},
		{"queryParam", queryParam.String()},
		{"callOrIndex", callOrIndex.String()},
		{"dottedName", dottedName.String()},
		{"shellVar", shellVar.String()},
		{"scriptVar", scriptVar.String()},
		{"identifier", identifier.String()},
		{"pemBegin", pemBegin.String()},
		{"pemEnd", pemEnd.String()},
	} {
		out = append(out, goldPattern{p.name, p.s})
	}
	for i, re := range strictPaths {
		out = append(out, goldPattern{fmt.Sprintf("strictPaths[%d]", i), re.String()})
	}
	goldScrubWrite(t, "patterns.json", out)
}

type goldScrubCase struct {
	Exact []string `json:"exact"`
	In    string   `json:"in"`
	Out   string   `json:"out"`
}

func goldScrubInputs() []string {
	v := "hunter2hunter2"
	in := []string{
		fakeAnthropic, fakeOpenAI, fakeOpenProj, fakeGHP, fakeGHO, fakeGHS, fakeGHU, fakeGHPat,
		fakeSlackB, fakeSlackP, fakeAWS, fakeAWSTemp, fakeGoogle, fakeJWT,
		"https://" + fakeGHP + "@github.com/o/r.git",
		"cookie: s=" + fakeJWT + "; Path=/",
		fakePEM("RSA ", "\n", 3, ""), fakePEM("", "\r\n", 2, ""), fakePEM("OPENSSH ", "\n", 2, "+"),
		"x\n-----BEGIN RSA PRIVATE KEY-----\nMIIEow\nMIIEow",
		"-----BEGIN ENCRYPTED PRIVATE KEY BLOCK-----\nabc\n-----END ENCRYPTED PRIVATE KEY BLOCK-----\n",
		"Authorization: Bearer " + fakeBearer, "Bearer\t" + fakeBearer + "==",
		"Authorization: Basic dXNlcjpwYXNzd29yZA==", "Authorization: Negotiate YIIabc123456",
		`authorization\":\"token ` + fakeBearer,
		"password=" + v, "token := `" + v + "`", `{\"password\":\"` + v + `\"}`,
		"https://h/cb?access_token=" + v + "&state=1", "postgres://app:" + v + "@db:5432/x",
		"https://user:p@ss0rdlong@host/x", `if password == "hunter2hunter2" {`,
		"password == hunter2hunter2", "token = *opts.Token", "token = *opts.Value",
		"secret: import.meta.env.VITE_SECRET,", "token: cfg.auth.value", "TOKEN=$(cat x)",
		samplePatch(),
		// Non-ASCII letters next to tokens: RE2's \b is ASCII, so these
		// are word boundaries in Go.
		"é" + fakeAnthropic, "ü" + fakeGHP + "ß", "日本" + fakeAWS + "語", "ñ" + fakeJWT,
		"ßBearer " + fakeBearer, "éBearer " + fakeBearer, "Ωauthorization: token " + fakeBearer,
		"ÿsk-" + strings.Repeat("Ab3d", 12), "AIza" + strings.Repeat("é", 35),
		"passwordé=" + v, "tokenü: " + v, "pässword=" + v,
		// \v and other spaces: not RE2 \s.
		"password=\v" + v, "password\v=" + v, "Bearer\v" + fakeBearer, "password= " + v + "\v",
		"https://user:pass\vword@h", "token=" + v + "\u00a0tail", "token=\u00a0" + v,
		"secret=" + v + "\u2028more",
		// Unicode case folds in (?i) and the ASCII gates.
		"PASSWORD=" + v, "Pa\u017fsword=" + v, "TO\u212aEN=" + v, "BEARER " + fakeBearer,
		"bearer " + fakeBearer, "AUTHORIZATION: BASIC abcdefgh1234", "\u212aey=" + v,
		"api_KEY=" + v, "API-Key: " + v, "apiKey=" + v, "api key=" + v,
		// Assorted edges.
		"password=" + v + "\r", "\r\npassword=" + v + "\r\n", "token=a\nb=" + v,
		"password: '" + v + "\\'more'", `password: "abc\"defghij"`,
		"token='" + v, `token="` + v, "x" + fakeGHP + "y", fakeGHP + fakeGHP,
		"sk-" + strings.Repeat("a", 30), "sk-" + strings.Repeat("a", 29) + "1",
		"redis://[redacted:exact]:" + v + "@h", "ftp://a:b@c", "a://:" + v + "@h",
		"password=[redacted:github]tail", "secret=\"[redacted:exact]xxxx\"",
		"password=abc&def=ghijklmnop", "token=abcdefgh;&x=1", "token=abcdefgh\\",
		"", "\n", "\r\n\r\n", "plain text\nwith lines\n",
	}
	return in
}

func TestGoldenScrubCases(t *testing.T) {
	var out []goldScrubCase
	none := New()
	for _, in := range goldScrubInputs() {
		out = append(out, goldScrubCase{[]string{}, in, none.String(in)})
	}
	lock := "irori-lock-" + strings.Repeat("7f", 12)
	exactSets := [][]string{
		{lock, "", "short", "attach-" + strings.Repeat("Q", 10)},
		{fakeGHP},
		{"abcdefgh", "abcdefghijkl", "efghijklmn"},
		{"line-one-value\r\nline-two-value"},
		{"ééééééé", "日本語日本語"},
	}
	exactInputs := []string{
		"token in file: " + lock + "\nsee attach-QQQQQQQQQQ",
		lock + lock, "x " + fakeGHP, "abcdefghijkl abcdefgh abcdefghijklmn",
		"abcdabcdefghijklmnop", "a line-one-value\r\nline-two-value b",
		"ééééééé日本語日本語é", "short",
	}
	for _, set := range exactSets {
		s := New(set...)
		for _, in := range exactInputs {
			out = append(out, goldScrubCase{set, in, s.String(in)})
		}
	}
	goldScrubWrite(t, "cases.json", out)
}

type goldBoolCase struct {
	In   string `json:"in"`
	Want bool   `json:"want"`
}

type goldStrictCase struct {
	In  string `json:"in"`
	Out string `json:"out"`
}

func TestGoldenScrubSkipFileStrict(t *testing.T) {
	var skips []goldBoolCase
	for _, p := range []string{
		".env", ".ENV", "a/.env", `C:\r\.env`, ".env.local", ".envrc", "x.pem", "x.PEM.", "x.pem. .",
		"x.key", "x.p12", "x.pfx", "id_rsa", "id_rsa.pub", "id_ed25519", "id_ecdsa_sk", "credentials",
		"Credentials.json", ".netrc", ".npmrc", ".pypirc", ".npmrc.bak", "env.go", "key.go",
		".env::$DATA", "x.pem:Zone.Identifier", "C:.env", "C:key.pem", "C:", "..", "", "dir/",
		`dir\`, "id_rsa ", " .env", "\u00a0.env", ".env\u00a0", "\u3000id_rsa", "İd_rsa", "ID_RSA",
		".ENV.İ", "x.pem\u200b", "notes.txt:secret.pem", "1:.env", "é:.env",
	} {
		skips = append(skips, goldBoolCase{p, SkipFile(p)})
	}
	goldScrubWrite(t, "skipfile.json", skips)

	var stricts []goldStrictCase
	for _, in := range []string{
		`fix C:\Users\pat\projects\irori\main.go now`, `see c:/work/x`, `under \\nas\share\docs`,
		`in ~/projects/mlg`, `in ~`, `at $HOME/.config/x`, `at ${HOME}/x`, `at %USERPROFILE%\AppData`,
		`at %HOMEPATH%`, `/Users/pat/projects/mlg/x.go breaks`, `log ("/home/me/logs/a.txt")`,
		`path=/var/lib/irori/x`, "see `/Users/pat/projects/x`", `file:///home/me/x.txt`,
		`open FILE://nas/share/x.txt now`, `at:/home/me/x`, `*/home/me/x*`, `(/Users/me/x)`,
		`[~/notes]`, "**/etc/irori/conf**", "`\\\\nas\\share\\x`", `edit src/coord/impl.go`,
		`https://github.com/StephenSHorton/irori/pull/12`, `read and/or write 1/2 of it`, `use /help`,
		"é/home/me/x", "éC:\\x", "日本C:/x", "\u00a0/home/me/x", "\v/home/me/x", "a\v~/x",
		"C:\\a C:\\b", "~ ~ ~", "/a/b/c /d/e", "x=C:\\y,z", "ÄC:\\x", "\u212afile://x",
		"see /home/me/é/x", "C:\\Users\\é\u00a0x", "/a/\u2028b/c", "", "plain",
	} {
		stricts = append(stricts, goldStrictCase{in, Strict(in)})
	}
	goldScrubWrite(t, "strict.json", stricts)
}

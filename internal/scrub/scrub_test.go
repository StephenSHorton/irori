package scrub

import (
	"encoding/base64"
	"fmt"
	"strings"
	"sync"
	"testing"
	"time"
)

// Fake tokens are built by concatenation so the repo never holds a string a
// secret scanner would flag. None of these is a real credential.
var (
	fakeAnthropic = "sk-" + "ant-" + "api03-" + strings.Repeat("a1B2", 10)
	fakeOpenAI    = "sk-" + strings.Repeat("Ab3d", 12)
	fakeOpenProj  = "sk-" + "proj-" + strings.Repeat("Xy9_", 20)
	fakeGHP       = "gh" + "p_" + strings.Repeat("A1b2C3", 6)
	fakeGHO       = "gh" + "o_" + strings.Repeat("Z9y8X7", 6)
	fakeGHS       = "gh" + "s_" + strings.Repeat("q1W2e3", 6)
	fakeGHU       = "gh" + "u_" + strings.Repeat("r4T5y6", 6)
	fakeGHPat     = "github" + "_pat_" + strings.Repeat("11AB", 6) + "_" + strings.Repeat("cd34", 14)
	fakeSlackB    = "xo" + "xb-" + "1234567890-1234567890123-" + strings.Repeat("aB3", 8)
	fakeSlackP    = "xo" + "xp-" + "1234567890-" + strings.Repeat("Zz9", 6)
	fakeAWS       = "AK" + "IA" + "IOSFODNN7" + "FAKE012"
	fakeAWSTemp   = "AS" + "IA" + strings.Repeat("Q7", 8)
	fakeGoogle    = "AI" + "za" + "Sy" + strings.Repeat("D-x_9", 6) + "abc"
	fakeJWT       = "ey" + "JhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9" + "." + "ey" + "JzdWIiOiIxMjM0NTY3ODkwIn0" + "." + strings.Repeat("Sf1Kx", 8)
	fakeBearer    = "abc123" + strings.Repeat("XyZ.-_", 5)
)

func fakePEM(kind, nl string, bodyLines int, prefix string) string {
	var b strings.Builder
	b.WriteString(prefix + "-----BEGIN " + kind + "PRIVATE KEY-----" + nl)
	for i := 0; i < bodyLines; i++ {
		b.WriteString(prefix + strings.Repeat(fmt.Sprintf("M%dIEvQ", i), 10) + nl)
	}
	b.WriteString(prefix + "-----END " + kind + "PRIVATE KEY-----")
	return b.String()
}

func lines(s string) (n, r int) { return strings.Count(s, "\n"), strings.Count(s, "\r") }

func check(t *testing.T, s *Scrubber, in, want string) {
	t.Helper()
	got := s.String(in)
	if got != want {
		t.Errorf("String(%q)\n got %q\nwant %q", in, got, want)
	}
	n, r := lines(got)
	if wn, wr := lines(in); n != wn || r != wr {
		t.Errorf("String(%q): line breaks \\n %d->%d, \\r %d->%d", in, wn, n, wr, r)
	}
	if again := s.String(got); again != got {
		t.Errorf("not idempotent: %q -> %q", got, again)
	}
}

func TestTokenShapes(t *testing.T) {
	s := New()
	cases := []struct{ in, want string }{
		{fakeAnthropic, "[redacted:anthropic]"},
		{"key " + fakeAnthropic + " end", "key [redacted:anthropic] end"},
		{fakeOpenAI, "[redacted:openai]"},
		{fakeOpenProj, "[redacted:openai]"},
		{"use " + fakeOpenAI + ".", "use [redacted:openai]."},
		{fakeGHP, "[redacted:github]"},
		{fakeGHO, "[redacted:github]"},
		{fakeGHS, "[redacted:github]"},
		{fakeGHU, "[redacted:github]"},
		{fakeGHPat, "[redacted:github]"},
		{"https://" + fakeGHP + "@github.com/o/r.git", "https://[redacted:github]@github.com/o/r.git"},
		{fakeSlackB, "[redacted:slack]"},
		{fakeSlackP, "[redacted:slack]"},
		{fakeAWS, "[redacted:aws]"},
		{"id=" + fakeAWSTemp + ";", "id=[redacted:aws];"},
		{fakeGoogle, "[redacted:google]"},
		{"?key=" + fakeGoogle + "&x=1", "?key=[redacted:google]&x=1"},
		{fakeJWT, "[redacted:jwt]"},
		{"cookie: s=" + fakeJWT + "; Path=/", "cookie: s=[redacted:jwt]; Path=/"},
		{"a " + fakeGHP + " b " + fakeSlackB + " c", "a [redacted:github] b [redacted:slack] c"},
		{`"` + fakeAnthropic + `"`, `"[redacted:anthropic]"`},
	}
	for _, c := range cases {
		check(t, s, c.in, c.want)
	}
}

func TestPrivateKey(t *testing.T) {
	s := New()
	cases := []struct {
		name, in, want string
	}{
		{"rsa lf", fakePEM("RSA ", "\n", 3, ""), "[redacted:private-key]\n\n\n\n"},
		{"plain crlf", fakePEM("", "\r\n", 2, ""), "[redacted:private-key]\r\n\r\n\r\n"},
		{"ec in text", "before\n" + fakePEM("EC ", "\n", 1, "") + "\nafter", "before\n[redacted:private-key]\n\n\nafter"},
		{"openssh", fakePEM("OPENSSH ", "\n", 2, ""), "[redacted:private-key]\n\n\n"},
		{"patch lines keep +", "@@ -0,0 +1,4 @@\n" + fakePEM("RSA ", "\n", 2, "+") + "\n context",
			"@@ -0,0 +1,4 @@\n+[redacted:private-key]\n+\n+\n+\n context"},
		{"patch removal lines keep -", fakePEM("", "\n", 1, "-"), "-[redacted:private-key]\n-\n-"},
		{"indented yaml", "key: |\n" + fakePEM("", "\n", 1, "  ") + "\nnext: 1",
			"key: |\n  [redacted:private-key]\n  \n  \nnext: 1"},
		{"json escaped one line", `{"private_key":"-----BEGIN PRIVATE KEY-----\nMIIEvQIBADAN\n-----END PRIVATE KEY-----\n"}`,
			`{"private_key":"[redacted:private-key]\n"}`},
		{"truncated, no END", "x\n-----BEGIN RSA PRIVATE KEY-----\nMIIEow\nMIIEow", "x\n[redacted:private-key]\n\n"},
		{"two blocks", fakePEM("", "\n", 1, "") + "\nmid\n" + fakePEM("", "\n", 1, ""),
			"[redacted:private-key]\n\n\nmid\n[redacted:private-key]\n\n"},
		{"public key untouched", "-----BEGIN PUBLIC KEY-----\nMIIBIjAN\n-----END PUBLIC KEY-----",
			"-----BEGIN PUBLIC KEY-----\nMIIBIjAN\n-----END PUBLIC KEY-----"},
		{"certificate untouched", "-----BEGIN CERTIFICATE-----\nMIIBIjAN\n-----END CERTIFICATE-----",
			"-----BEGIN CERTIFICATE-----\nMIIBIjAN\n-----END CERTIFICATE-----"},
		{"prose", "never paste your PRIVATE KEY here", "never paste your PRIVATE KEY here"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) { check(t, s, c.in, c.want) })
	}
}

func TestBearer(t *testing.T) {
	s := New()
	cases := []struct{ in, want string }{
		{"Authorization: Bearer " + fakeBearer, "Authorization: Bearer [redacted:bearer]"},
		{"authorization: bearer " + fakeBearer, "authorization: bearer [redacted:bearer]"},
		{`curl -H "Authorization: Bearer ` + fakeBearer + `" https://x`, `curl -H "Authorization: Bearer [redacted:bearer]" https://x`},
		{`{"Authorization": "Bearer ` + fakeBearer + `"}`, `{"Authorization": "Bearer [redacted:bearer]"}`},
		{"Bearer\t" + fakeBearer + "==", "Bearer\t[redacted:bearer]"},
		{"Authorization: Bearer " + fakeJWT, "Authorization: Bearer [redacted:jwt]"},
		{"Authorization: Basic " + base64.StdEncoding.EncodeToString([]byte("user:password")), "Authorization: Basic [redacted:bearer]"},
		{"Authorization: token " + fakeBearer, "Authorization: token [redacted:bearer]"},
		// not tokens
		{"Authorization: Bearer $TOKEN", "Authorization: Bearer $TOKEN"},
		{"Authorization: Bearer ${TOKEN}", "Authorization: Bearer ${TOKEN}"},
		{"Bearer abc", "Bearer abc"},
		{"the bearer instruments rose", "the bearer instruments rose"},
		{"Bearer\nabcdef123456789", "Bearer\nabcdef123456789"}, // never across a line break
	}
	for _, c := range cases {
		check(t, s, c.in, c.want)
	}
}

func TestAssignment(t *testing.T) {
	s := New()
	const v = "hunter2hunter2"
	cases := []struct{ in, want string }{
		{"password=" + v, "password=[redacted:assignment]"},
		{"password = " + v, "password = [redacted:assignment]"},
		{"password: " + v, "password: [redacted:assignment]"},
		{`password: "` + v + `"`, `password: "[redacted:assignment]"`},
		{`password = '` + v + `'`, `password = '[redacted:assignment]'`},
		{"token := `" + v + "`", "token := `[redacted:assignment]`"},
		{`"password": "` + v + `"`, `"password": "[redacted:assignment]"`},
		{`{"password":"` + v + `","user":"bob"}`, `{"password":"[redacted:assignment]","user":"bob"}`},
		{`{\"password\":\"` + v + `\"}`, `{\"password\":\"[redacted:assignment]\"}`},
		{`'client_secret' => '` + v + `',`, `'client_secret' => '[redacted:assignment]',`},
		{"DB_PASSWORD=" + v, "DB_PASSWORD=[redacted:assignment]"},
		{"export API_TOKEN=" + v, "export API_TOKEN=[redacted:assignment]"},
		{`export API_KEY="` + v + `"`, `export API_KEY="[redacted:assignment]"`},
		{"set APIKEY=" + v, "set APIKEY=[redacted:assignment]"},
		{"$env:MY_SECRET = '" + v + "'", "$env:MY_SECRET = '[redacted:assignment]'"},
		{"passwd: " + v, "passwd: [redacted:assignment]"},
		{"api-key: " + v, "api-key: [redacted:assignment]"},
		{"aws_secret_access_key = " + strings.Repeat("fAkE7", 8), "aws_secret_access_key = [redacted:assignment]"},
		{"x-access-token: " + v, "x-access-token: [redacted:assignment]"},
		{"https://h/cb?access_token=" + v + "&state=1", "https://h/cb?access_token=[redacted:assignment]&state=1"},
		{"--password=" + v, "--password=[redacted:assignment]"},
		{"password=12345678", "password=[redacted:assignment]"}, // digits leak just as well
		{"Password: " + v, "Password: [redacted:assignment]"},
		{`SECRET="a b c d e f"`, `SECRET="[redacted:assignment]"`},
		{`token = "` + fakeGHP + `"`, `token = "[redacted:github]"`},
		{"a=1\r\nTOKEN=" + v + "\r\nb=2", "a=1\r\nTOKEN=[redacted:assignment]\r\nb=2"},
		{"token=" + v + " secret=" + v, "token=[redacted:assignment] secret=[redacted:assignment]"},
		{"password: 'abc\\'defghij'", "password: '[redacted:assignment]'"},
		{`"secret": "abc\"defghij"`, `"secret": "[redacted:assignment]"`},
		{"https://[redacted:github]:" + "pw12345678@h", "https://[redacted:github]:[redacted:assignment]@h"},
		// URLs with user:password@
		{"postgres://app:" + v + "@db:5432/x", "postgres://app:[redacted:assignment]@db:5432/x"},
		{"https://user:pw@example.com/", "https://user:[redacted:assignment]@example.com/"},
		{"redis://default:" + v + "@cache", "redis://default:[redacted:assignment]@cache"},
	}
	for _, c := range cases {
		check(t, s, c.in, c.want)
	}
}

func TestEnvFile(t *testing.T) {
	s := New()
	in := "# app settings\r\n" +
		"PORT=8080\r\n" +
		"DATABASE_URL=postgres://app:s3cretpass@localhost/app\r\n" +
		"STRIPE_SECRET_KEY=" + "rk_" + strings.Repeat("live9", 6) + "\r\n" +
		"GITHUB_TOKEN=" + fakeGHP + "\r\n" +
		"SESSION_SECRET=\r\n" +
		"DEBUG=true\r\n"
	want := "# app settings\r\n" +
		"PORT=8080\r\n" +
		"DATABASE_URL=postgres://app:[redacted:assignment]@localhost/app\r\n" +
		"STRIPE_SECRET_KEY=[redacted:assignment]\r\n" +
		"GITHUB_TOKEN=[redacted:github]\r\n" +
		"SESSION_SECRET=\r\n" +
		"DEBUG=true\r\n"
	check(t, s, in, want)
}

// TestFalsePositives: ordinary code and prose must come through unchanged.
func TestFalsePositives(t *testing.T) {
	s := New()
	plainB64 := base64.StdEncoding.EncodeToString([]byte("the quick brown fox jumps over the lazy dog, twice over"))
	jsonB64 := base64.RawURLEncoding.EncodeToString([]byte(`{"name":"irori","kind":"hub"}`))
	cases := []string{
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
		`"password": "hunter"`,
		"AKIA is the prefix for access key ids",
		"AIza prefix",
		"eyJ is how base64 JSON starts",
		jsonB64,
		plainB64,
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
	}
	for _, in := range cases {
		check(t, s, in, in)
	}
}

func TestExact(t *testing.T) {
	lock := "irori-lock-" + strings.Repeat("7f", 12)
	attach := "attach-" + strings.Repeat("Q", 10)
	s := New(lock, "", "short", attach)
	check(t, s, "token in file: "+lock+"\nsee "+attach, "token in file: [redacted:exact]\nsee [redacted:exact]")
	check(t, s, "short stays", "short stays")
	check(t, s, lock+lock, "[redacted:exact][redacted:exact]")

	// Exact wins over shape, so Irori's own value is tagged exact.
	s.SetExact(fakeGHP)
	check(t, s, "x "+fakeGHP, "x [redacted:exact]")

	// SetExact replaces the list.
	s.SetExact(attach)
	check(t, s, lock, lock)
	check(t, s, attach, "[redacted:exact]")

	// Longest value wins when one contains another.
	s.SetExact("abcdefgh", "abcdefghijkl")
	check(t, s, "abcdefghijkl abcdefgh", "[redacted:exact] [redacted:exact]")

	// A multi-line exact value keeps its line breaks.
	s.SetExact("line-one-value\r\nline-two-value")
	check(t, s, "a line-one-value\r\nline-two-value b", "a [redacted:exact]\r\n b")

	s.SetExact()
	check(t, s, attach, attach)

	check(t, New(), "nothing here", "nothing here")
}

func TestConcurrent(t *testing.T) {
	s := New("value-zero-0000")
	var wg sync.WaitGroup
	for g := 0; g < 8; g++ {
		wg.Add(2)
		go func(g int) {
			defer wg.Done()
			for i := 0; i < 200; i++ {
				s.SetExact(fmt.Sprintf("value-%d-%04d", g, i), "value-zero-0000")
			}
		}(g)
		go func() {
			defer wg.Done()
			for i := 0; i < 200; i++ {
				if got := s.String("x value-zero-0000 y"); got != "x [redacted:exact] y" {
					t.Errorf("got %q", got)
					return
				}
			}
		}()
	}
	wg.Wait()
}

// TestLineCount: every rule on a mixed, multi-line patch keeps the line count
// and never grows a line by more than one marker per secret.
func TestLineCount(t *testing.T) {
	s := New()
	p := samplePatch()
	out := s.String(p)
	in, gotN := strings.Split(p, "\n"), strings.Split(out, "\n")
	if len(in) != len(gotN) {
		t.Fatalf("lines %d -> %d", len(in), len(gotN))
	}
	if out == p {
		t.Fatal("sample patch had nothing redacted")
	}
	for i := range in {
		if len(gotN[i]) > len(in[i])+len("[redacted:private-key]") {
			t.Errorf("line %d grew: %q -> %q", i, in[i], gotN[i])
		}
	}
	for _, leak := range []string{fakeAnthropic, fakeGHP, fakeAWS, fakeJWT, "hunter2hunter2", "M0IEvQ"} {
		if strings.Contains(out, leak) {
			t.Errorf("leaked %q", leak)
		}
	}
}

func samplePatch() string {
	return "--- a/config/app.go\n" +
		"+++ b/config/app.go\n" +
		"@@ -1,6 +1,14 @@\n" +
		" package config\n" +
		"-const maxTokens = 2048\n" +
		"+const maxTokens = 4096\n" +
		"+var anthropicKey = \"" + fakeAnthropic + "\"\n" +
		"+var gh = \"" + fakeGHP + "\" // " + fakeAWS + "\n" +
		"+// Authorization: Bearer " + fakeJWT + "\n" +
		"+password := \"hunter2hunter2\"\n" +
		fakePEM("RSA ", "\n", 3, "+") + "\n" +
		" func tokenCount() int { return 0 }\n"
}

func bigPatch() string {
	var b strings.Builder
	chunk := samplePatch()
	plain := strings.Repeat(" func (s *Store) Token() string { return s.tokenCount }\n", 20)
	for b.Len() < 4<<20 {
		b.WriteString(chunk)
		b.WriteString(plain)
	}
	return b.String()
}

func TestBigPatchTime(t *testing.T) {
	s := New("irori-lock-" + strings.Repeat("ab", 12))
	p := bigPatch()
	start := time.Now()
	out := s.String(p)
	if d := time.Since(start); d > slow*10*time.Second {
		t.Errorf("4 MiB patch took %v", d)
	}
	if strings.Count(out, "\n") != strings.Count(p, "\n") {
		t.Error("line count changed")
	}
	if strings.Contains(out, fakeGHP) {
		t.Error("leaked")
	}
}

// TestLongLineTime: one huge line (minified JSON) has no line splits to help.
func TestLongLineTime(t *testing.T) {
	s := New()
	var b strings.Builder
	for b.Len() < 1<<20 {
		b.WriteString(`{"tokenCount":12,"password":"hunter2hunter2","note":"Bearer abc","k":"` + fakeGHP + `"},`)
	}
	p := b.String()
	start := time.Now()
	out := s.String(p)
	if d := time.Since(start); d > slow*10*time.Second {
		t.Errorf("1 MiB line took %v", d)
	}
	if strings.Contains(out, fakeGHP) || strings.Contains(out, "hunter2hunter2") {
		t.Error("leaked")
	}
}

func TestContainsAnyFold(t *testing.T) {
	words := []string{"token", "api_key"}
	cases := []struct {
		s    string
		want bool
	}{
		{"TOKEN", true},
		{"myToken=1", true},
		{"API_KEY", true},
		{"apiKey", false},
		{"tok", false},
		{"toke", false},
		{"", false},
		{"API\x7fKEY", false},
	}
	for _, c := range cases {
		if got := containsAnyFold(c.s, words); got != c.want {
			t.Errorf("containsAnyFold(%q) = %v", c.s, got)
		}
	}
}

func BenchmarkString4MiB(b *testing.B) {
	s := New("irori-lock-" + strings.Repeat("ab", 12))
	p := bigPatch()
	b.SetBytes(int64(len(p)))
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		s.String(p)
	}
}

func BenchmarkStringClean4MiB(b *testing.B) {
	s := New()
	p := strings.Repeat(" func (s *Store) Get(k string) (string, bool) { return s.m[k] }\n", (4<<20)/64)
	b.SetBytes(int64(len(p)))
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		s.String(p)
	}
}

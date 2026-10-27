package scrub

import "testing"

// TestReviewFindings pins the leaks found in review: each input must come
// back with the secret redacted.
func TestReviewFindings(t *testing.T) {
	s := New()
	discord := "MTk4NjIyNDgzNDcxOTI1MjQ4" + "." + "Cl2FMQ" + "." + "ZnCjm1XVW7vRze4b7Cq4se7kKWs"
	cases := []struct{ name, in, want string }{
		// mixed quotes inside a quoted value
		{"apostrophe in double quotes", `"password": "it's-a-secret-value-123"`, `"password": "[redacted:assignment]"`},
		{"double quote in single quotes", `token = 'abc"defghijklmnop'`, `token = '[redacted:assignment]'`},
		{"backtick in double quotes", "secret = \"abc`defghijkl\"", `secret = "[redacted:assignment]"`},
		{"quote in escaped json", `{\"password\":\"it's-a-secret-1\"}`, `{\"password\":\"[redacted:assignment]\"}`},
		// dotted tokens
		{"discord env", "DISCORD_TOKEN=" + discord, "DISCORD_TOKEN=[redacted:assignment]"},
		{"dotted letters", "token: abcdefgh.ijklmnop.qrstuvwx", "token: [redacted:assignment]"},
		// punctuation inside unquoted values
		{"ampersand", "DB_PASSWORD=p&ssw0rd!longvalue", "DB_PASSWORD=[redacted:assignment]"},
		{"semicolon", "DB_PASSWORD=abc;def1234567890", "DB_PASSWORD=[redacted:assignment]"},
		{"comma", "api_key=abc,defghijklmnopq", "api_key=[redacted:assignment]"},
		{"pipe", "export TOKEN=abc|defghijklmnop", "export TOKEN=[redacted:assignment]"},
		{"parens", "SECRET_KEY=django-insecure-(abc)defghijk", "SECRET_KEY=[redacted:assignment]"},
		{"backslash", `password=abc\defghijkl`, `password=[redacted:assignment]`},
		{"trailing punctuation kept", "f(token=abcdefghijkl);", "f(token=[redacted:assignment]);"},
		{"query param cut", "?token=abc&defghijkl&next=1", "?token=[redacted:assignment]&next=1"},
		{"escaped quote end", `"cmd":"export TOKEN=abcdefghijkl\" ok"`, `"cmd":"export TOKEN=[redacted:assignment]\" ok"`},
		{"base64 padding", "secret=QUJDREVGR0hJSg==", "secret=[redacted:assignment]"},
		{"object literal", "{ password: abcd1234efgh }", "{ password: [redacted:assignment] }"},
		// leading sigils
		{"dollar", "PASSWORD=$ecretPassw0rd99", "PASSWORD=[redacted:assignment]"},
		{"at", "password: @dm1nPassw0rd!", "password: [redacted:assignment]"},
		{"star", "password: *Hunter2Hunter2", "password: [redacted:assignment]"},
		{"percent", "password=%Passw0rd123", "password=[redacted:assignment]"},
		// URL password with @
		{"url at in password", "https://user:p@ss0rdlong@host/x", "https://user:[redacted:assignment]@host/x"},
		// comparisons
		{"double equals", `if password == "hunter2hunter2" {`, `if password == "[redacted:assignment]" {`},
		{"triple equals", `if (token === 'hunter2hunter2') {`, `if (token === '[redacted:assignment]') {`},
		{"not equals", `if secret != "hunter2hunter2" {`, `if secret != "[redacted:assignment]" {`},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) { check(t, s, c.in, c.want) })
	}
}

// TestReviewFalsePositives: code the fixes above must still leave alone.
func TestReviewFalsePositives(t *testing.T) {
	s := New()
	cases := []string{
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
	}
	for _, in := range cases {
		check(t, s, in, in)
	}
}

func TestZeroScrubber(t *testing.T) {
	var z Scrubber
	if got := z.String("password=hunter2hunter2"); got != "password=[redacted:assignment]" {
		t.Errorf("zero String = %q", got)
	}
	var n *Scrubber
	if got := n.String("x " + fakeGHP); got != "x [redacted:github]" {
		t.Errorf("nil String = %q", got)
	}
	z.SetExact("my-exact-value-1")
	if got := z.String("a my-exact-value-1"); got != "a [redacted:exact]" {
		t.Errorf("zero SetExact then String = %q", got)
	}
}

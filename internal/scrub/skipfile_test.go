package scrub

import "testing"

func TestSkipFile(t *testing.T) {
	cases := []struct {
		path string
		want bool
	}{
		{".env", true},
		{".ENV", true},
		{"app/.env", true},
		{`C:\repo\app\.env`, true},
		{".env.local", true},
		{"web/.env.production", true},
		{".env.example", true},
		{"server.pem", true},
		{`certs\Server.PEM`, true},
		{"tls/private.key", true},
		{"store.p12", true},
		{"store.PFX", true},
		{"id_rsa", true},
		{"id_rsa.pub", true},
		{"~/.ssh/id_ed25519", true},
		{`C:\Users\me\.ssh\id_ecdsa`, true},
		{"credentials", true},
		{"credentials.json", true},
		{"aws/Credentials", true},
		{".netrc", true},
		{"home/.npmrc", true},
		{".PYPIRC", true},
		{"a/b\\c/.env", true},
		// not secrets
		{"env.go", false},
		{"dotenv.go", false},
		{".envrc", false}, // contract: .env and .env.* only
		{"environment.ts", false},
		{"key.go", false},
		{"keys/README.md", false},
		{"my.pem.bak", false},
		{"pem/readme.txt", false},
		{".env/readme.md", false}, // a directory named .env; base name is readme.md
		{"tokens.go", false},
		{"rsa_id", false},
		{"my_credentials.go", false},
		{".npmrc.bak", false},
		{".netrc.d/x", false},
		{"", false},
		{"dir/", false},
		{`dir\`, false},
	}
	for _, c := range cases {
		if got := SkipFile(c.path); got != c.want {
			t.Errorf("SkipFile(%q) = %v, want %v", c.path, got, c.want)
		}
	}
}

// TestSkipFileWindowsNames: Win32 drops trailing dots and spaces, and an
// alternate data stream names the same file.
func TestSkipFileWindowsNames(t *testing.T) {
	cases := []struct {
		path string
		want bool
	}{
		{"key.pem.", true},
		{"key.pem. .", true},
		{"id_rsa ", true},
		{".env::$DATA", true},
		{"x.pem:Zone.Identifier", true},
		{`C:\repo\.env:stream:$DATA`, true},
		{"C:.env", true},
		{"C:key.pem", true},
		{`C:\repo\notes.txt:secret.pem`, false}, // the file is notes.txt
		{"..", false},
		{"C:", false},
		{"readme.md.", false},
	}
	for _, c := range cases {
		if got := SkipFile(c.path); got != c.want {
			t.Errorf("SkipFile(%q) = %v, want %v", c.path, got, c.want)
		}
	}
}

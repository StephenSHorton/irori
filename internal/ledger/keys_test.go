package ledger

import "testing"

func TestKey(t *testing.T) {
	tests := []struct {
		repo string
		n    int
		want string
	}{
		{"acme/webapp", 1437, "pr:acme/webapp#1437"},
		{"Acme/WebApp", 12, "pr:acme/webapp#12"},
		{" acme/webapp ", 1, "pr:acme/webapp#1"},
	}
	for _, tt := range tests {
		if got := Key(tt.repo, tt.n); got != tt.want {
			t.Errorf("Key(%q, %d) = %q, want %q", tt.repo, tt.n, got, tt.want)
		}
	}
}

func TestRepoKey(t *testing.T) {
	tests := []struct{ repo, want string }{
		{"acme/webapp", "remote:github.com/acme/webapp"},
		{"Acme/WebApp", "remote:github.com/acme/webapp"},
	}
	for _, tt := range tests {
		if got := RepoKey(tt.repo); got != tt.want {
			t.Errorf("RepoKey(%q) = %q, want %q", tt.repo, got, tt.want)
		}
	}
}

func TestRepoFromKey(t *testing.T) {
	tests := []struct{ key, want string }{
		{"remote:github.com/acme/webapp", "acme/webapp"},
		{RepoKey("Acme/WebApp"), "acme/webapp"},
		{"remote:gitlab.com/acme/webapp", ""},
		{"local:C:/Users/sam/projects/webapp", ""},
		{"remote:github.com/acme", ""},
		{"remote:github.com/acme/", ""},
		{"remote:github.com//webapp", ""},
		{"remote:github.com/acme/webapp/extra", ""},
		{"Remote:github.com/acme/webapp", ""},
		{"", ""},
	}
	for _, tt := range tests {
		if got := RepoFromKey(tt.key); got != tt.want {
			t.Errorf("RepoFromKey(%q) = %q, want %q", tt.key, got, tt.want)
		}
	}
}

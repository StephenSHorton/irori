package repo

import "testing"

func TestNormalizeRemote(t *testing.T) {
	cases := []struct{ in, want string }{
		// The three spellings from the contract.
		{"git@github.com:Foo/Bar.git", "github.com/foo/bar"},
		{"https://github.com/Foo/Bar", "github.com/foo/bar"},
		{"ssh://git@github.com/foo/bar.git", "github.com/foo/bar"},

		// scp-like.
		{"github.com:owner/repo", "github.com/owner/repo"},
		{"git@GitHub.COM:Owner/Repo", "github.com/owner/repo"},
		{"  git@github.com:o/n.git\n", "github.com/o/n"},
		{"user@host.example:~user/proj/name.git", "host.example/~user/proj/name"},

		// http(s) with user, password, port, .git, trailing slash.
		{"https://user:pw@Host.example:8443/Owner/Name.git/", "host.example/owner/name"},
		{"http://host.example/o/n/", "host.example/o/n"},
		{"https://github.com/o/n.git", "github.com/o/n"},
		{"https://github.com//o//n", "github.com/o/n"},
		{"https://github.com/o/my%20repo", "github.com/o/my repo"},
		{"https://github.com/o/n?x=1#frag", "github.com/o/n"},
		{"HTTPS://GITHUB.COM/O/N", "github.com/o/n"},

		// ssh://, git://, git+ssh://.
		{"ssh://git@host.example:2222/o/n.git", "host.example/o/n"},
		{"ssh://host.example/o/n", "host.example/o/n"},
		{"git://host.example/o/n", "host.example/o/n"},
		{"git+ssh://git@github.com/o/n.git", "github.com/o/n"},

		// GitLab subgroups keep the full path.
		{"https://gitlab.com/group/sub/sub2/name.git", "gitlab.com/group/sub/sub2/name"},
		{"git@gitlab.com:Group/Sub/Name.git", "gitlab.com/group/sub/name"},

		// Azure DevOps: every spelling folds onto the https shape.
		{"https://org@dev.azure.com/Org/Proj/_git/Repo", "dev.azure.com/org/proj/_git/repo"},
		{"https://dev.azure.com/org/proj/_git/repo", "dev.azure.com/org/proj/_git/repo"},
		{"git@ssh.dev.azure.com:v3/Org/Proj/Repo", "dev.azure.com/org/proj/_git/repo"},
		{"ssh://git@ssh.dev.azure.com/v3/org/proj/repo", "dev.azure.com/org/proj/_git/repo"},
		{"org@vs-ssh.visualstudio.com:v3/org/proj/repo", "dev.azure.com/org/proj/_git/repo"},
		{"https://org.visualstudio.com/proj/_git/repo", "dev.azure.com/org/proj/_git/repo"},
		{"https://org.visualstudio.com/DefaultCollection/proj/_git/repo", "dev.azure.com/org/proj/_git/repo"},

		// Local paths.
		{"/abs/path/repo.git", ""},
		{"./rel", ""},
		{"../rel/repo", ""},
		{"~/src/repo", ""},
		{`C:\src\repo`, ""},
		{"C:/src/repo", ""},
		{"c:", ""},
		{`\\server\share\repo`, ""},
		{"file:///home/me/repo.git", ""},
		{"file://C:/src/repo", ""},

		// Unparseable or too short.
		{"", ""},
		{"   ", ""},
		{"repo", ""},
		{"dir/a:b", ""},
		{"https://github.com/", ""},
		{"https://github.com/onlyname", ""},
		{"git@github.com:name.git", ""},
		{"git@github.com:", ""},
		{"https://github.com/o/../n", ""},
		{"ftp://host.example/o/n", ""},
		{"https:///o/n", ""},
	}
	for _, c := range cases {
		if got := NormalizeRemote(c.in); got != c.want {
			t.Errorf("NormalizeRemote(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

func TestSmallestSHA(t *testing.T) {
	if got := smallestSHA("ffff\n\nAAAA\nbbbb\n"); got != "aaaa" {
		t.Fatalf("got %q", got)
	}
	if got := smallestSHA("\n \n"); got != "" {
		t.Fatalf("got %q", got)
	}
}

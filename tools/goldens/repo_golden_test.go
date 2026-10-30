// Golden exporter for crates/capture's repo module (capture.md §0 I8,
// §3 "repo keys", §7 trap 9).
//
// Injected into I:internal/repo by tools/goldens/export.sh with
// `go test -overlay`; runs only when IRORI_GOLDEN_OUT is set and writes
// $IRORI_GOLDEN_OUT/capture/repo.json: NormalizeRemote over the Go test
// table plus inputs aimed at net/url's corners, and smallestSHA.
package repo

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestGoldenRepo(t *testing.T) {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	ins := []string{
		"git@github.com:Foo/Bar.git", "https://github.com/Foo/Bar", "ssh://git@github.com/foo/bar.git",
		"github.com:owner/repo", "git@GitHub.COM:Owner/Repo", "  git@github.com:o/n.git\n",
		"user@host.example:~user/proj/name.git", "https://user:pw@Host.example:8443/Owner/Name.git/",
		"http://host.example/o/n/", "https://github.com/o/n.git", "https://github.com//o//n",
		"https://github.com/o/my%20repo", "https://github.com/o/n?x=1#frag", "HTTPS://GITHUB.COM/O/N",
		"ssh://git@host.example:2222/o/n.git", "ssh://host.example/o/n", "git://host.example/o/n",
		"git+ssh://git@github.com/o/n.git", "https://gitlab.com/group/sub/sub2/name.git", "git@gitlab.com:Group/Sub/Name.git",
		"https://org@dev.azure.com/Org/Proj/_git/Repo", "https://dev.azure.com/org/proj/_git/repo",
		"git@ssh.dev.azure.com:v3/Org/Proj/Repo", "ssh://git@ssh.dev.azure.com/v3/org/proj/repo",
		"org@vs-ssh.visualstudio.com:v3/org/proj/repo", "https://org.visualstudio.com/proj/_git/repo",
		"https://org.visualstudio.com/DefaultCollection/proj/_git/repo",
		"ssh://org@org.visualstudio.com:22/DefaultCollection/proj/_ssh/repo", "ssh://org@org.visualstudio.com:22/proj/_ssh/repo",
		"https://github.com/o/_ssh/n", "/abs/path/repo.git", "./rel", "../rel/repo", "~/src/repo", `C:\src\repo`,
		"C:/src/repo", "c:", `\\server\share\repo`, "file:///home/me/repo.git", "file://C:/src/repo", "", "   ", "repo",
		"dir/a:b", "https://github.com/", "https://github.com/onlyname", "git@github.com:name.git", "git@github.com:",
		"https://github.com/o/../n", "ftp://host.example/o/n", "https:///o/n", "https://github.com/o/n%0Ax",
		"https://github.com/o/n%00x", "https://github.com/o/n%2Fx", "https://github.com/o%2fn", "https://github.com/o/n%5Cx",
		"https://github.com/o/%2E%2E/n", "git@github.com:o/n\x01",
		// net/url corners.
		"https://github.com/o/n%zz", "https://github.com/o/n#%zz", "https://git hub.com/o/n", "https://[::1]:22/o/n",
		"https://[::1/o/n", "https://host:port/o/n", "https://host:/o/n", "https://u%zz@host/o/n", "https://u^x@host/o/n",
		"https://host/o/n b", "https://host/o/ü", "https://host/o/%C3%BC", "https://host/o/n;x", "https://host/o/n?",
		"https://host/o/n??x", "https://host./o/n", "https://HOST.example./O/N.GIT", "https:github.com/o/n",
		"git+https://host/o/n", "ssh+git://host/o/n", "1https://host/o/n", "://host/o/n", "https://host/o/.git",
		"https://host/o/n.git.git", "https://host/o/n/.git", "https://ho%41st/o/n", "https://host/%2e/n", "https://host/o/%2e%2e",
		"https://user@host@x/o/n", "git@host:o/n/", "@host:o/n", "h:o/n", "hh:o/n", "git@[::1]:o/n", "git@host:/o/n",
		"git@host:o//n", "https://host/o/n\t", "https://host/o/\u0085x", "ssh://git@ssh.dev.azure.com/v3/org/proj",
		"https://x.visualstudio.com/", "https://x.visualstudio.com/defaultcollection", "https://host/a/_ssh/b",
		"https://dev.azure.com/o/p/_ssh/r", "HTTPS://Host/O/N", "git@host:o/N.Git", "https://host/o/n%2",
		"https://host/o/n%%41", "https://host/~o/n", "https://host/o/n#", "https://host:8080", "https://host:8080/o",
	}
	type nv struct{ In, Out string }
	var out []nv
	for _, in := range ins {
		out = append(out, nv{in, NormalizeRemote(in)})
	}
	v := map[string]any{
		"normalizeRemote": out,
		"smallestSHA": []nv{{"ffff\n\nAAAA\nbbbb\n", smallestSHA("ffff\n\nAAAA\nbbbb\n")},
			{"\n \n", smallestSHA("\n \n")}, {" b \n A ", smallestSHA(" b \n A ")}},
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(v); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "capture", "repo.json")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

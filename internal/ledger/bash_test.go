package ledger

import (
	"reflect"
	"testing"
)

func TestFromBash(t *testing.T) {
	const url = "https://github.com/acme/webapp/pull/1437"
	created := []byte(`{"pr":{"number":1437,"url":"` + url + `","action":"created"}}`)
	author := func(repo string, n int, u, source string) BashRef {
		return BashRef{Ref: Ref{repo, n}, URL: u, Role: Author, Source: source}
	}
	reviewer := func(repo string, n int, u string) BashRef {
		return BashRef{Ref: Ref{repo, n}, URL: u, Role: Reviewer, Source: "gh"}
	}
	tests := []struct {
		name string
		b    BashResult
		want []BashRef
	}{
		{
			name: "gitOperation created",
			b:    BashResult{Command: `gh pr create --title "x" --body "y"`, Stdout: url + "\n", GitOperation: created},
			want: []BashRef{author(web, 1437, url, "gitOperation")},
		},
		{
			name: "gitOperation created beside push, any command",
			b:    BashResult{Command: "git push -u origin feat/x && gh pr create --fill", GitOperation: []byte(`{"push":{"branch":"feat/x"},"pr":{"number":1437,"url":"` + url + `","action":"created"}}`)},
			want: []BashRef{author(web, 1437, url, "gitOperation")},
		},
		{
			name: "gitOperation wins over stdout",
			b:    BashResult{Command: "gh pr create --fill", Stdout: "https://github.com/acme/other/pull/5\n", GitOperation: created},
			want: []BashRef{author(web, 1437, url, "gitOperation")},
		},
		{
			name: "gitOperation without url takes DefaultRepo",
			b:    BashResult{Command: "gh pr create", GitOperation: []byte(`{"pr":{"number":9,"action":"created"}}`), DefaultRepo: web},
			want: []BashRef{author(web, 9, "", "gitOperation")},
		},
		{
			name: "gitOperation without url or repo is skipped",
			b:    BashResult{Command: "echo hi", GitOperation: []byte(`{"pr":{"number":9,"action":"created"}}`)},
			want: nil,
		},
		{
			name: "gitOperation without number takes it from the url",
			b:    BashResult{Command: "echo hi", GitOperation: []byte(`{"pr":{"url":"` + url + `","action":"created"}}`)},
			want: []BashRef{author(web, 1437, url, "gitOperation")},
		},
		{
			name: "gitOperation edited is not authorship",
			b:    BashResult{Command: "gh pr edit 1437 --title x", GitOperation: []byte(`{"pr":{"number":1437,"url":"` + url + `","action":"edited"}}`)},
			want: nil,
		},
		{
			name: "gitOperation merged is not authorship",
			b:    BashResult{Command: "gh pr merge 1437", GitOperation: []byte(`{"pr":{"number":1437,"action":"merged"}}`), DefaultRepo: web},
			want: nil,
		},
		{
			name: "edited gitOperation falls back to gh pr create stdout",
			b:    BashResult{Command: "gh pr create --fill", Stdout: url + "\n", GitOperation: []byte(`{"pr":{"number":1,"action":"edited"}}`)},
			want: []BashRef{author(web, 1437, url, "gh")},
		},
		{
			name: "bad gitOperation json falls back to stdout",
			b:    BashResult{Command: "gh pr create --fill", Stdout: url, GitOperation: []byte(`{"pr":`)},
			want: []BashRef{author(web, 1437, url, "gh")},
		},
		{
			name: "gh pr create stdout, first url only",
			b:    BashResult{Command: "gh pr create --fill", Stdout: "Creating pull request\r\n" + url + "\r\nhttps://github.com/acme/webapp/pull/1438\r\n"},
			want: []BashRef{author(web, 1437, url, "gh")},
		},
		{
			name: "gh pr create with extra spaces",
			b:    BashResult{Command: "cd C:/src/webapp && gh  pr   create --fill", Stdout: url},
			want: []BashRef{author(web, 1437, url, "gh")},
		},
		{
			name: "stdout url without gh pr create is ignored",
			b:    BashResult{Command: "gh pr view 1437", Stdout: url},
			want: nil,
		},
		{
			name: "gh pr created is not gh pr create",
			b:    BashResult{Command: "gh pr created", Stdout: url},
			want: nil,
		},
		{
			name: "gh pr create without a url",
			b:    BashResult{Command: "gh pr create --fill", Stdout: "a pull request for branch already exists"},
			want: nil,
		},
		{
			name: "review by url",
			b:    BashResult{Command: "gh pr review " + url + " --approve"},
			want: []BashRef{reviewer(web, 1437, url)},
		},
		{
			name: "review by url without scheme",
			b:    BashResult{Command: "gh pr review github.com/acme/webapp/pull/12/files --comment -b ok"},
			want: []BashRef{reviewer(web, 12, "https://github.com/acme/webapp/pull/12/files")},
		},
		{
			name: "review by number with -R",
			b:    BashResult{Command: "gh pr review 12 -R acme/storefront --approve", DefaultRepo: web},
			want: []BashRef{reviewer("acme/storefront", 12, "")},
		},
		{
			name: "review by number with --repo=",
			b:    BashResult{Command: "gh pr review 12 --repo=acme/storefront --approve"},
			want: []BashRef{reviewer("acme/storefront", 12, "")},
		},
		{
			name: "review by number with --repo before the number",
			b:    BashResult{Command: "gh pr review --repo acme/storefront 12 --approve"},
			want: []BashRef{reviewer("acme/storefront", 12, "")},
		},
		{
			name: "review by number with -R before the number",
			b:    BashResult{Command: "gh pr review -R acme/storefront 12"},
			want: []BashRef{reviewer("acme/storefront", 12, "")},
		},
		{
			name: "review by quoted number",
			b:    BashResult{Command: `gh pr review "12" --approve`, DefaultRepo: web},
			want: []BashRef{reviewer(web, 12, "")},
		},
		{
			name: "review by number takes DefaultRepo",
			b:    BashResult{Command: "gh pr review 12 --approve", DefaultRepo: web},
			want: []BashRef{reviewer(web, 12, "")},
		},
		{
			name: "review by number without a repo is skipped",
			b:    BashResult{Command: "gh pr review 12 --approve"},
			want: nil,
		},
		{
			name: "review with a flag first is skipped",
			b:    BashResult{Command: "gh pr review --approve 12", DefaultRepo: web},
			want: nil,
		},
		{
			name: "review of a branch name is skipped",
			b:    BashResult{Command: "gh pr review feat/x --approve", DefaultRepo: web},
			want: nil,
		},
		{
			name: "every review, -R stops at the command separator",
			b:    BashResult{Command: "gh pr review 12 --approve && gh pr review 13 -R acme/storefront; gh pr review 14 | cat", DefaultRepo: web},
			want: []BashRef{reviewer(web, 12, ""), reviewer("acme/storefront", 13, ""), reviewer(web, 14, "")},
		},
		{
			name: "reviews across CRLF lines",
			b:    BashResult{Command: "gh pr review 12 --approve\r\ngh pr review 13 -R acme/storefront\r\n", DefaultRepo: web},
			want: []BashRef{reviewer(web, 12, ""), reviewer("acme/storefront", 13, "")},
		},
		{
			name: "the same review twice is one",
			b:    BashResult{Command: "gh pr review 12 --comment -b a; gh pr review 12 --approve", DefaultRepo: web},
			want: []BashRef{reviewer(web, 12, "")},
		},
		{
			name: "create and review together",
			b:    BashResult{Command: "gh pr create --fill && gh pr review 1437 --comment -b self", Stdout: url, DefaultRepo: web},
			want: []BashRef{author(web, 1437, url, "gh"), reviewer(web, 1437, "")},
		},
		{
			name: "review number zero is skipped",
			b:    BashResult{Command: "gh pr review 0", DefaultRepo: web},
			want: nil,
		},
		{name: "nothing", b: BashResult{Command: "go test ./..."}, want: nil},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := FromBash(tt.b); !reflect.DeepEqual(got, tt.want) {
				t.Errorf("FromBash = %+v, want %+v", got, tt.want)
			}
		})
	}
}

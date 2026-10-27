package ledger

import (
	"reflect"
	"testing"
)

const web = "acme/webapp"

func refKeys(refs []Ref) []string {
	out := []string{}
	for _, r := range refs {
		out = append(out, Key(r.Repo, r.Number))
	}
	return out
}

func TestExtractRefs(t *testing.T) {
	aliases := map[string]string{"web": "webapp", "shop": "acme/storefront"}
	tests := []struct {
		name string
		text string
		o    RefOptions
		want []string
	}{
		// Ported from tsukumo refs.test.ts.
		{
			name: "URLs, repo#N, aliases and bare #N in order, without repeats",
			text: "APPROVED webapp#1407 (3ff5049c7). See https://github.com/acme/webapp/pull/1407 and WEB#1408, SHOP#498, api-server#143, then #1414.",
			o:    RefOptions{DefaultRepo: web, Aliases: aliases},
			want: []string{"pr:acme/webapp#1407", "pr:acme/webapp#1408", "pr:acme/storefront#498", "pr:acme/api-server#143", "pr:acme/webapp#1414"},
		},
		{name: "bare #N needs a default repo", text: "REVIEW-PLEASE #492", want: []string{}},
		{
			name: "channel and role mentions are not PRs",
			text: "<#100000000000000001> <@&100000000000000014> <@100000000000000015> hi",
			o:    RefOptions{DefaultRepo: web}, want: []string{},
		},
		{name: "bare #N with a default repo", text: "REVIEW-PLEASE #492", o: RefOptions{DefaultRepo: web}, want: []string{"pr:acme/webapp#492"}},
		{name: "owner/repo#N", text: "APPROVED acme/webapp#1407", want: []string{"pr:acme/webapp#1407"}},
		{name: "PR#N and pull#N", text: "PR#1407 and pull#1408", o: RefOptions{DefaultRepo: web}, want: []string{"pr:acme/webapp#1407", "pr:acme/webapp#1408"}},
		{
			name: "code spans and long numbers",
			text: "see `#12` and ```\n#13\n``` and order #333333",
			o:    RefOptions{DefaultRepo: web}, want: []string{},
		},
		{name: "a repo without an owner needs one", text: "webapp#12", o: RefOptions{Aliases: map[string]string{}}, want: []string{}},
		{name: "explicit owner", text: "webapp#12", o: RefOptions{Owner: "acme"}, want: []string{"pr:acme/webapp#12"}},

		// Irori edge cases.
		{name: "PR#N without default repo is skipped", text: "PR#12", want: []string{}},
		{name: "owner beats default repo owner", text: "api#7", o: RefOptions{DefaultRepo: web, Owner: "other"}, want: []string{"pr:other/api#7"}},
		{name: "default repo owner for name#N", text: "api#7", o: RefOptions{DefaultRepo: web}, want: []string{"pr:acme/api#7"}},
		{name: "alias with owner needs no owner", text: "shop#5", o: RefOptions{Aliases: aliases}, want: []string{"pr:acme/storefront#5"}},
		{name: "alias without owner needs one", text: "web#5", o: RefOptions{Aliases: aliases}, want: []string{}},
		{name: "alias lookup is case-insensitive", text: "Web#5", o: RefOptions{Owner: "acme", Aliases: aliases}, want: []string{"pr:acme/webapp#5"}},
		{name: "repo case is kept, key dedupes", text: "Acme/WebApp#5 acme/webapp#5", want: []string{"pr:acme/webapp#5"}},
		{name: "name#N 1 digit", text: "acme/webapp#1", want: []string{"pr:acme/webapp#1"}},
		{name: "name#N 7 digits is not a ref", text: "acme/webapp#1234567", want: []string{}},
		{name: "bare #N 1 digit is not a ref", text: "#7", o: RefOptions{DefaultRepo: web}, want: []string{}},
		{name: "bare #N 5 digits", text: "#12345", o: RefOptions{DefaultRepo: web}, want: []string{"pr:acme/webapp#12345"}},
		{name: "bare #N 6 digits is not a ref", text: "#123456", o: RefOptions{DefaultRepo: web}, want: []string{}},
		{name: "digits followed by a word char", text: "#12a acme/webapp#3x", o: RefOptions{DefaultRepo: web}, want: []string{}},
		{name: "digits followed by punctuation", text: "(#12), acme/webapp#3.", o: RefOptions{DefaultRepo: web}, want: []string{"pr:acme/webapp#12", "pr:acme/webapp#3"}},
		{name: "a word before # makes name#N", text: "abc#12", o: RefOptions{DefaultRepo: web}, want: []string{"pr:acme/abc#12"}},
		{name: "bare #N after a word char only", text: "1#12 _#13", o: RefOptions{DefaultRepo: web}, want: []string{}},
		{name: "bare #N after slash, <, #, &, @", text: "/#12 <#13 ##14 &#15 @#16", o: RefOptions{DefaultRepo: web}, want: []string{}},
		{name: "bare #N after dash is allowed", text: "-#12", o: RefOptions{DefaultRepo: web}, want: []string{"pr:acme/webapp#12"}},
		{name: "name#N after dash is not a ref", text: "x -webapp#12", o: RefOptions{Owner: "acme"}, want: []string{}},
		{name: "name#N after slash, <, # is not a ref", text: "a/b/c#12 <web#13 #web#14", o: RefOptions{Owner: "acme"}, want: []string{}},
		{name: "name#N after non-ASCII", text: "éwebapp#12", o: RefOptions{Owner: "acme"}, want: []string{"pr:acme/webapp#12"}},
		{name: "name#N after & or @ is allowed", text: "&webapp#12 @api#13", o: RefOptions{Owner: "acme"}, want: []string{"pr:acme/webapp#12", "pr:acme/api#13"}},
		{name: "name must start with a letter", text: "1webapp#12 _x#13", o: RefOptions{Owner: "acme"}, want: []string{}},
		{name: "a dash inside a name is part of it", text: "x-webapp#12 then #14", o: RefOptions{DefaultRepo: web}, want: []string{"pr:acme/x-webapp#12", "pr:acme/webapp#14"}},
		{
			// The lookbehind rejects "/a#12"; the scan goes on and finds b#13.
			name: "rejected match retries at the next position",
			text: "/a#12 b#13",
			o:    RefOptions{Owner: "acme"}, want: []string{"pr:acme/b#13"},
		},
		{
			name: "rejected name start does not swallow a URL",
			text: "-github.com/acme/webapp/pull/9",
			want: []string{"pr:acme/webapp#9"},
		},
		{name: "URL without scheme", text: "github.com/acme/webapp/pull/77/files", want: []string{"pr:acme/webapp#77"}},
		{name: "issue URL is not a PR", text: "https://github.com/acme/webapp/issues/77", want: []string{}},
		{name: "URL number too big", text: "github.com/acme/webapp/pull/99999999999999999999999", want: []string{}},
		{name: "zero is not a PR", text: "acme/webapp#0 #00", o: RefOptions{DefaultRepo: web}, want: []string{}},
		{name: "inline code across a newline is not code", text: "`a\n#12 b`", o: RefOptions{DefaultRepo: web}, want: []string{"pr:acme/webapp#12"}},
		{name: "fenced code with CRLF", text: "```go\r\n#12\r\n```\r\n#13\r\n", o: RefOptions{DefaultRepo: web}, want: []string{"pr:acme/webapp#13"}},
		{name: "unclosed fence is prose", text: "```\n#12", o: RefOptions{DefaultRepo: web}, want: []string{"pr:acme/webapp#12"}},
		{name: "empty", text: "", o: RefOptions{DefaultRepo: web}, want: []string{}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := refKeys(ExtractRefs(tt.text, tt.o)); !reflect.DeepEqual(got, tt.want) {
				t.Errorf("ExtractRefs(%q) = %v, want %v", tt.text, got, tt.want)
			}
		})
	}
}

func TestExtractRefsKeepsRepoCase(t *testing.T) {
	got := ExtractRefs("Acme/WebApp#5 and #60", RefOptions{DefaultRepo: "Acme/WebApp"})
	want := []Ref{{Repo: "Acme/WebApp", Number: 5}, {Repo: "Acme/WebApp", Number: 60}}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %+v, want %+v", got, want)
	}
}

func TestParseTarget(t *testing.T) {
	tests := []struct {
		in   string
		want Ref
		ok   bool
	}{
		{"pr:acme/webapp#1437", Ref{web, 1437}, true},
		{"PR:acme/webapp#1437", Ref{web, 1437}, true},
		{"acme/webapp#1437", Ref{web, 1437}, true},
		{"Acme/WebApp#3", Ref{"Acme/WebApp", 3}, true},
		{"  acme/webapp#3\r\n", Ref{web, 3}, true},
		{"https://github.com/acme/webapp/pull/1437", Ref{web, 1437}, true},
		{"http://github.com/acme/webapp/pull/1437", Ref{web, 1437}, true},
		{"github.com/acme/webapp/pull/1437", Ref{web, 1437}, true},
		{"https://www.github.com/acme/webapp/pull/1437", Ref{web, 1437}, true},
		{"https://GitHub.com/acme/webapp/pull/1437", Ref{web, 1437}, true},
		{"https://github.com/acme/webapp/pull/1437/files", Ref{web, 1437}, true},
		{"https://github.com/acme/webapp/pull/1437/", Ref{web, 1437}, true},
		{"https://github.com/acme/webapp/pull/1437?diff=split", Ref{web, 1437}, true},
		{"https://github.com/acme/webapp/pull/1437#issuecomment-1", Ref{web, 1437}, true},
		{"https://github.com/acme/my.repo-x/pull/9", Ref{"acme/my.repo-x", 9}, true},
		{"pr:acme#12", Ref{}, false},
		{"webapp#12", Ref{}, false},
		{"#12", Ref{}, false},
		{"acme/webapp#12x", Ref{}, false},
		{"acme/webapp#12 extra", Ref{}, false},
		{"acme/webapp#0", Ref{}, false},
		{"https://github.com/acme/webapp/pull/", Ref{}, false},
		{"https://github.com/acme/webapp/pull/12x", Ref{}, false},
		{"https://github.com/acme/webapp/issues/12", Ref{}, false},
		{"https://gitlab.com/acme/webapp/pull/12", Ref{}, false},
		{"https://evilgithub.com/acme/webapp/pull/12", Ref{}, false},
		{"see https://github.com/acme/webapp/pull/12", Ref{}, false},
		{"pr:https://github.com/acme/webapp/pull/12", Ref{}, false},
		{"", Ref{}, false},
	}
	for _, tt := range tests {
		got, ok := ParseTarget(tt.in)
		if ok != tt.ok || got != tt.want {
			t.Errorf("ParseTarget(%q) = %+v, %v; want %+v, %v", tt.in, got, ok, tt.want, tt.ok)
		}
	}
}

// Golden exporter for crates/ledger (store.md §8, §7.12).
//
// Injected into I:internal/ledger by tools/goldens/export.sh with
// `go test -overlay`; runs only when IRORI_GOLDEN_OUT is set. It records
// ExtractRefs, ParseTarget, FromBash and the keys over the Go test tables'
// inputs plus adversarial extras (\v, Unicode folds, non-ASCII neighbours,
// gitOperation decoding), Entry decoding, and a scripted run of Add and the
// three views with every result marshaled by Go.
//
// Byte strings that may hold invalid UTF-8 are written as lowercase hex.
package ledger_test

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/rand/v2"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/ledger"
)

func ledgerGoldenDir(t *testing.T) string {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	return dir
}

func writeLedgerGolden(t *testing.T, rel string, v any) {
	path := filepath.Join(ledgerGoldenDir(t), rel)
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
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

const gWeb = "acme/webapp"
const gURL = "https://github.com/acme/webapp/pull/1437"

// refTexts: every TestExtractRefs text, then extras.
var refTexts = []string{
	"APPROVED webapp#1407 (3ff5049c7). See https://github.com/acme/webapp/pull/1407 and WEB#1408, SHOP#498, api-server#143, then #1414.",
	"REVIEW-PLEASE #492",
	"<#100000000000000001> <@&100000000000000014> <@100000000000000015> hi",
	"APPROVED acme/webapp#1407",
	"acme/..#12 and github.com/../x/pull/3 then acme/webapp#4",
	"PR#1407 and pull#1408",
	"see `#12` and ```\n#13\n``` and order #333333",
	"webapp#12",
	"PR#12",
	"api#7",
	"shop#5",
	"web#5",
	"Web#5",
	"Acme/WebApp#5 acme/webapp#5",
	"acme/webapp#1",
	"acme/webapp#1234567",
	"#7",
	"#12345",
	"#123456",
	"#12a acme/webapp#3x",
	"(#12), acme/webapp#3.",
	"abc#12",
	"1#12 _#13",
	"/#12 <#13 ##14 &#15 @#16",
	"-#12",
	"x -webapp#12",
	"a/b/c#12 <web#13 #web#14",
	"éwebapp#12",
	"&webapp#12 @api#13",
	"1webapp#12 _x#13",
	"x-webapp#12 then #14",
	"/a#12 b#13",
	"-github.com/acme/webapp/pull/9",
	"github.com/acme/webapp/pull/77/files",
	"https://github.com/acme/webapp/issues/77",
	"github.com/acme/webapp/pull/99999999999999999999999",
	"acme/webapp#0 #00",
	"`a\n#12 b`",
	"```go\r\n#12\r\n```\r\n#13\r\n",
	"```\n#12",
	"",
	"Acme/WebApp#5 and #60",
	// Extras.
	"#12\v#13\v acme/webapp#14\v",
	"\v#12 x\vwebapp#13",
	"#12é #13ü #14\u00a0 #15\u2028",
	"é#12 ü#13 中#14 中webapp#15",
	"Straße#4 naïve#5 Ǆ#6",
	"İstanbul#12 ıx#13 ſhop#14 \u212aelvin#15",
	"PULL#12 Pr#13 pR#14 pUlL#15 prx#16 pr/x#17",
	"github.com/\u212aelvin/x/pull/5 github.com/ſ/x/pull/6",
	"GITHUB.COM/acme/webapp/pull/1 Github.com/acme/webapp/pull/2",
	"https://github.com/a/b/pull/007 github.com/a/b/pull/0 github.com/a/b/pull/12#13",
	"acme/webapp#000123 acme/webapp#0000001",
	"``` ``` #12 ```` #13 ````",
	"`#12`#13 ``#14`` `x`y`#15`",
	"```\n#12\n``` ```\n#13",
	"x#12#13 a#1#2 a.b#3 a..#4 .a#5 a.#6",
	"o/r/x#1 o/r#2/x o//r#3 o/.#4 ./r#5 o/r.#6",
	"web#1 WEB#2 Web#3 shop#4 SHOP#5",
	"a_b#12 a-b-c#13 A9#14 Z_#15",
	"\t#12\n#13\r#14\f#15 #16",
	"<#12> <@#13> &#14; #&15",
	"github.com/acme/webapp/pull/12github.com/acme/webapp/pull/13",
	"ggithub.com/acme/webapp/pull/14 xgithub.com/acme/webapp/pull/15",
	"#99999 #100000 a/b#999999 a/b#1000000",
	"emoji 🙂#12 🙂webapp#13",
}

var refOptionSets = []struct {
	Name        string            `json:"name"`
	DefaultRepo string            `json:"defaultRepo"`
	Owner       string            `json:"owner"`
	Aliases     map[string]string `json:"aliases"`
}{
	{Name: "none"},
	{Name: "default", DefaultRepo: gWeb},
	{Name: "owner+aliases", Owner: "acme", Aliases: map[string]string{"web": "webapp", "shop": "acme/storefront", "ſhop": "x/y"}},
	{Name: "default+owner+aliases", DefaultRepo: "Acme/WebApp", Owner: "other", Aliases: map[string]string{"web": "webapp", "shop": "acme/storefront"}},
	{Name: "default-no-slash", DefaultRepo: "acme"},
}

var targets = []string{
	"pr:acme/webapp#1437", "PR:acme/webapp#1437", "acme/webapp#1437", "Acme/WebApp#3", "  acme/webapp#3\r\n",
	"https://github.com/acme/webapp/pull/1437", "http://github.com/acme/webapp/pull/1437", "github.com/acme/webapp/pull/1437",
	"https://www.github.com/acme/webapp/pull/1437", "https://GitHub.com/acme/webapp/pull/1437",
	"https://github.com/acme/webapp/pull/1437/files", "https://github.com/acme/webapp/pull/1437/",
	"https://github.com/acme/webapp/pull/1437?diff=split", "https://github.com/acme/webapp/pull/1437#issuecomment-1",
	"https://github.com/acme/my.repo-x/pull/9", "pr:acme#12", "webapp#12", "#12", "acme/webapp#12x", "acme/webapp#12 extra",
	"acme/webapp#0", "https://github.com/acme/webapp/pull/", "https://github.com/acme/webapp/pull/12x",
	"https://github.com/acme/webapp/issues/12", "https://gitlab.com/acme/webapp/pull/12",
	"https://evilgithub.com/acme/webapp/pull/12", "see https://github.com/acme/webapp/pull/12",
	"pr:https://github.com/acme/webapp/pull/12", "", "../..#1", "pr:acme/..#1", "./webapp#1",
	"https://github.com/../../pull/1", "https://github.com/acme/.../pull/1", "acme/.github#4", "acme/web..app#4",
	// Extras.
	"\vacme/webapp#3\v", "\u0085acme/webapp#3\u00a0", "\u3000acme/webapp#3\u2029", "\u200bacme/webapp#3",
	"https://github.com/\u212a/x/pull/1", "https://github.com/ſ/x/pull/1", "https://github.com/a/\u212aelvin/pull/2",
	"https://GİTHUB.com/a/b/pull/1", "https://github.\u212aom/a/b/pull/1", "HTTPS://WWW.GITHUB.COM/a/b/PULL/1",
	"httpſ://github.com/a/b/pull/1", "pR:acme/webapp#1", "Pr:acme/webapp#1", "pr:", "pr", "pr:pr:acme/webapp#1",
	"pr:acme/webapp#99999999999999999999", "github.com/a/b/pull/99999999999999999999", "acme/webapp#9223372036854775807",
	"acme/webapp#9223372036854775808", "github.com/a/b/pull/1\n", "https://github.com/a/b/pull/1/x\ny",
	"https://github.com/a/b/pull/1/x\vy", "acme/webapp#+5", "acme/webapp#007", "acme/ webapp#1", "acme/webapp# 1",
	"é/webapp#1", "acme/web_app#1", "a/b#1#2", "github.com/a/b/pull/1#x\n", "github.com//b/pull/1",
	"https://github.com/a/b/pull/1?", "https://github.com/a/b/pull/1x/", "ftp://github.com/a/b/pull/1",
	"https:/github.com/a/b/pull/1", "www.github.com/a/b/pull/3",
}

type bashCase struct {
	Command     string `json:"command"`
	Stdout      string `json:"stdout"`
	GitOpHex    string `json:"gitOperationHex"`
	DefaultRepo string `json:"defaultRepo"`
}

func bashCases() []bashCase {
	created := `{"pr":{"number":1437,"url":"` + gURL + `","action":"created"}}`
	c := func(cmd, stdout, gitop, def string) bashCase {
		return bashCase{cmd, stdout, hex.EncodeToString([]byte(gitop)), def}
	}
	return []bashCase{
		// Every TestFromBash input.
		c(`gh pr create --title "x" --body "y"`, gURL+"\n", created, ""),
		c("git push -u origin feat/x && gh pr create --fill", "", `{"push":{"branch":"feat/x"},"pr":{"number":1437,"url":"`+gURL+`","action":"created"}}`, ""),
		c("gh pr create --fill", "https://github.com/acme/other/pull/5\n", created, ""),
		c("gh pr create", "", `{"pr":{"number":9,"action":"created"}}`, gWeb),
		c("echo hi", "", `{"pr":{"number":9,"action":"created"}}`, ""),
		c("echo hi", "", `{"pr":{"url":"`+gURL+`","action":"created"}}`, ""),
		c("gh pr edit 1437 --title x", "", `{"pr":{"number":1437,"url":"`+gURL+`","action":"edited"}}`, ""),
		c("gh pr merge 1437", "", `{"pr":{"number":1437,"action":"merged"}}`, gWeb),
		c("gh pr create --fill", gURL+"\n", `{"pr":{"number":1,"action":"edited"}}`, ""),
		c("gh pr create --fill", gURL, `{"pr":`, ""),
		c("gh pr create --fill", "Creating pull request\r\n"+gURL+"\r\nhttps://github.com/acme/webapp/pull/1438\r\n", "", ""),
		c("cd C:/src/webapp && gh  pr   create --fill", gURL, "", ""),
		c("gh pr view 1437", gURL, "", ""),
		c("gh pr created", gURL, "", ""),
		c("gh pr create --fill", "a pull request for branch already exists", "", ""),
		c("gh pr review "+gURL+" --approve", "", "", ""),
		c("gh pr review github.com/acme/webapp/pull/12/files --comment -b ok", "", "", ""),
		c("gh pr review 12 -R acme/storefront --approve", "", "", gWeb),
		c("gh pr review 12 --repo=acme/storefront --approve", "", "", ""),
		c("gh pr review --repo acme/storefront 12 --approve", "", "", ""),
		c("gh pr review -R acme/storefront 12", "", "", ""),
		c(`gh pr review "12" --approve`, "", "", gWeb),
		c("gh pr review 12 --approve", "", "", gWeb),
		c("gh pr review 12 --approve", "", "", ""),
		c("gh pr review feat/x --approve", "", "", gWeb),
		c("gh pr review 12 --approve && gh pr review 13 -R acme/storefront; gh pr review 14 | cat", "", "", gWeb),
		c("gh pr review 12 --approve\r\ngh pr review 13 -R acme/storefront\r\n", "", "", gWeb),
		c("gh pr review 12 -R github.com/acme/storefront --approve", "", "", gWeb),
		c("gh pr review 12 --repo GitHub.com/acme/storefront", "", "", gWeb),
		c("gh pr review 12 --repo=github.com/acme/storefront", "", "", gWeb),
		c("gh pr review -R github.com/acme/storefront 12", "", "", ""),
		c("gh pr review 12 -R ghe.example.com/acme/storefront", "", "", gWeb),
		c("gh pr review 12 -Racme/storefront", "", "", gWeb),
		c(`gh pr review 12 -R "acme/storefront"`, "", "", gWeb),
		c("gh pr review 12 -R acme", "", "", gWeb),
		c(`gh pr review 12 --body "ok; fine | done && more" -R acme/other`, "", "", "acme/def"),
		c(`gh pr review 12 --body "see --repo acme/x"`, "", "", "acme/def"),
		c(`gh pr review 12 -b 'use -R acme/x' --approve`, "", "", "acme/def"),
		c("gh pr review 12 --body-file x--repo", "", "", "acme/def"),
		c("gh pr review --approve 12", "", "", "acme/def"),
		c("gh pr review -a 12; gh pr review -c 13; gh pr review -r 14", "", "", "acme/def"),
		c("gh pr review --comment 12 && gh pr review --request-changes 13", "", "", "acme/def"),
		c(`gh pr review -c -b "looks good" 12`, "", "", "acme/def"),
		c("gh pr review --body 99 12 --comment", "", "", "acme/def"),
		c("gh pr review --body-file C:\\tmp\\r.md 12 && gh pr review -F ./r.md -R acme/x 13", "", "", "acme/def"),
		c(`gh pr review --body="a b" 12`, "", "", "acme/def"),
		c("gh pr review --approve -- 12", "", "", "acme/def"),
		c("gh pr review https://evilgithub.com/acme/webapp/pull/3 --approve", "", "", ""),
		c("gh pr review github.com/acme/webapp/pull/3abc --approve", "", "", ""),
		c("gh pr review HTTP://WWW.GitHub.com/acme/webapp/pull/3?x=1 --approve", "", "", ""),
		c("gh pr review https://github.com/../../pull/3", "", "", ""),
		c("gh pr review 3 -R ../..", "", "", gWeb),
		c("gh pr create --fill", "https://evilgithub.com/acme/webapp/pull/3\n", "", ""),
		c("gh pr create --fill", "https://github.com/acme/webapp/pull/1437/files\n", "", ""),
		c("gh pr create", "", `{"pr":{"number":9,"url":"https://evilgithub.com/x/y/pull/9","action":"created"}}`, gWeb),
		c(`gh pr review 12 --body "then run gh pr review 13" --comment`, "", "", "acme/def"),
		c(`gh pr review 12 -b "say \"hi; -R acme/x\"" -R acme/y`, "", "", "acme/def"),
		c("gh pr review --approve", "", "", "acme/def"),
		c("gh pr review 12 --comment -b a; gh pr review 12 --approve", "", "", gWeb),
		c("gh pr create --fill && gh pr review 1437 --comment -b self", gURL, "", gWeb),
		c("gh pr review 0", "", "", gWeb),
		c("go test ./...", "", "", ""),

		// \v is not \s in Go.
		c("gh\vpr\vcreate", gURL, "", ""),
		c("gh pr create\v--fill", gURL, "", ""),
		c("gh\tpr\ncreate", gURL, "", ""),
		c("gh\vpr\vreview 12", "", "", gWeb),
		c("gh pr review\v12", "", "", gWeb),
		c("gh pr review 12\v-R acme/x", "", "", gWeb),
		c("gh\tpr\f review\t12", "", "", gWeb),
		c("gh pr review", "", "", gWeb),
		c("gh pr review\n12", "", "", gWeb),
		// Word boundaries around gh.
		c("xgh pr review 12; _gh pr review 13; égh pr review 14; -gh pr review 15; .gh pr review 16", "", "", gWeb),
		c("GH PR CREATE", gURL, "", ""),
		c("gh pr createx", gURL, "", ""),
		c("gh pr create-x", gURL, "", ""),
		c("gh pr createé", gURL, "", ""),
		c("gh pr reviewer 12; gh pr review-x 13", "", "", gWeb),
		// Stdout URL scraping.
		c("gh pr create", "x.github.com/acme/webapp/pull/1 -github.com/acme/webapp/pull/2 _github.com/acme/webapp/pull/3", "", ""),
		c("gh pr create", "(https://github.com/acme/webapp/pull/4)", "", ""),
		c("gh pr create", "https://github.com/acme/webapp/pull/5é", "", ""),
		c("gh pr create", "https://github.com/acme/webapp/pull/6_", "", ""),
		c("gh pr create", "éhttps://github.com/acme/webapp/pull/7", "", ""),
		c("gh pr create", "https://github.com/\u212aelvin/webapp/pull/8", "", ""),
		c("gh pr create", "https://github.com/ſcme/webapp/pull/9", "", ""),
		c("gh pr create", "HTTPS://WWW.GITHUB.COM/Acme/WebApp/PULL/10", "", ""),
		c("gh pr create", "https://github.com/acme/webapp/pull/0 https://github.com/acme/webapp/pull/11", "", ""),
		c("gh pr create", "https://github.com/../webapp/pull/12 https://github.com/acme/webapp/pull/13", "", ""),
		c("gh pr create", "https://github.com/acme/webapp/pull/99999999999999999999", "", ""),
		c("gh pr create", "\vhttps://github.com/acme/webapp/pull/14", "", ""),
		c("gh pr create", "https://github.com/acme/webapp/pull/015", "", ""),
		// gitOperation decoding as json.Unmarshal does it.
		c("echo", "", `{"PR":{"NUMBER":5,"Url":"`+gURL+`","ACTION":"created"}}`, ""),
		c("echo", "", `{"pr":{"number":5,"action":"Created"}}`, gWeb),
		c("gh pr create", gURL, `{"pr":{"number":"5","action":"created"}}`, gWeb),
		c("gh pr create", gURL, `{"pr":{"number":5.0,"action":"created"}}`, gWeb),
		c("gh pr create", gURL, `{"pr":{"number":5e0,"action":"created"}}`, gWeb),
		c("gh pr create", gURL, `{"pr":{"number":1e30,"action":"created"}}`, gWeb),
		c("gh pr create", gURL, `{"pr":{"number":99999999999999999999,"action":"created"}}`, gWeb),
		c("gh pr create", gURL, `{"pr":null}`, gWeb),
		c("gh pr create", gURL, `null`, gWeb),
		c("gh pr create", gURL, `[]`, gWeb),
		c("gh pr create", gURL, `"x"`, gWeb),
		c("gh pr create", gURL, ` `, gWeb),
		c("echo", "", `{"pr":{"action":"created"},"pr":{"number":3}}`, gWeb),
		c("echo", "", `{"pr":{"number":3},"pr":null,"pr":{"action":"created"}}`, gWeb),
		c("echo", "", `{"pr":{"number":-3,"url":"`+gURL+`","action":"created"}}`, ""),
		c("echo", "", `{"pr":{"number":-3,"action":"created"}}`, gWeb),
		c("echo", "", "{\"pr\":{\"number\":4,\"url\":\"\xff\",\"action\":\"created\"}}", gWeb),
		c("echo", "", "{\"pr\":{\"number\":4,\"url\":\"x\",\"action\":\"creat\xffed\"}}", gWeb),
		c("echo", "", `{"pr":{"number":4,"action":"created","extra":[1,{"a":null}]},"push":7}`, gWeb),
		c("echo", "", `{"pr":{"number":4,"action":"created","url":null}}`, gWeb),
		c("echo", "", `{"pr":{"number":4,"action":"created","url":5}}`, gWeb),
		c("echo", "", `{"pr":{"number":4,"action":"cr\u0065ated","url":"https:\/\/github.com\/a\/b\/pull\/4"}}`, ""),
		c("echo", "", `{"pr":{"number":4,"action":"created"}} x`, gWeb),
		c("echo", "", `{"pr":{"number":4,"action":"created","url":"see https://github.com/a/b/pull/77 now"}}`, ""),
		c("echo", "", `{"pr":{"number":4,"action":"created","url":"https://github.com/a/b/pull/0"}}`, ""),
		c("echo", "", `{"pr":{"number":4,"action":"created","url":"https://github.com/a/b/pull/0"}}`, gWeb),
		c("echo", "", `{"pr":{"number":0,"action":"created","url":"https://github.com/a/b/pull/0"}}`, gWeb),
		c("echo", "", `{"pr":{"nUmBeR":8,"aCtIoN":"created"}}`, gWeb),
		c("echo", "", `{"pr":{"number":8,"action":"created"},"Pr":{"number":9}}`, gWeb),
		// Review argument parsing.
		c("gh pr review 12 -R www.github.com/a/b", "", "", gWeb),
		c("gh pr review 12 -R a/b/", "", "", gWeb),
		c("gh pr review 12 -R github.com/a/b/c", "", "", gWeb),
		c("gh pr review 12 -R /a/b", "", "", gWeb),
		c("gh pr review 12 -R a/b -R c/d", "", "", gWeb),
		c("gh pr review 12 -R a/b --repo=", "", "", gWeb),
		c("gh pr review 12 -R", "", "", gWeb),
		c("gh pr review 12 --repo", "", "", ""),
		c("gh pr review 12 -R=a/b", "", "", gWeb),
		c("gh pr review 12 --repo=a/b=c", "", "", gWeb),
		c("gh pr review 12 --Repo=a/b", "", "", gWeb),
		c("gh pr review 12 -r a/b", "", "", gWeb),
		c("gh pr review -- -5", "", "", gWeb),
		c("gh pr review -- --repo a/b 12", "", "", gWeb),
		c("gh pr review - 12", "", "", gWeb),
		c("gh pr review 'unterminated", "", "", gWeb),
		c(`gh pr review "12`, "", "", gWeb),
		c(`gh pr review "1"2 --approve`, "", "", gWeb),
		c(`gh pr review 1\2`, "", "", gWeb),
		c(`gh pr review "1\2"`, "", "", gWeb),
		c(`gh pr review "a\\" 12`, "", "", gWeb),
		c("gh pr review 12 #comment -R a/b", "", "", gWeb),
		c("gh pr review ١٢", "", "", gWeb),
		c("gh pr review ０１", "", "", gWeb),
		c("gh pr review 99999999999999999999", "", "", gWeb),
		c("gh pr review 007", "", "", gWeb),
		c("gh pr review +5", "", "", gWeb),
		c("gh pr review https://github.com/\u212aelvin/b/pull/5", "", "", ""),
		c("gh pr review github.com/a/b/pull/5\vx", "", "", ""),
		c("gh pr review 'github.com/a/b/pull/5 x'", "", "", ""),
		c("gh pr review github.com/a/b/pull/5/", "", "", ""),
		c("gh pr review www.github.com/a/b/pull/5", "", "", ""),
		c("gh pr review evil.com/github.com/a/b/pull/5", "", "", ""),
		c("gh pr review 12&&gh pr review 13||gh pr review 14|gh pr review 15", "", "", gWeb),
		c("gh pr review 12 -R a/b\ngh pr review 13", "", "", gWeb),
		c("gh pr review\tgh pr review 12", "", "", gWeb),
		c("gh pr review 'gh pr review 12' 13", "", "", gWeb),
		c("gh pr review 12 -b 'x' -R 'a/b'", "", "", ""),
		c("gh pr review ''", "", "", gWeb),
		c(`gh pr review "" 12`, "", "", gWeb),
		c("gh pr review 12 -R é/b", "", "", ""),
		c("gh pr review 12 -R GITHUB.COM/a/b", "", "", ""),
		c("gh pr review 12 -R \u212a.github.com/a/b", "", "", ""),
		c("gh pr create --fill; gh pr review 1437", gURL, "", "acme/other"),
		c("gh pr create --fill", gURL, created, gWeb),
	}
}

var keyRepos = []string{
	"acme/webapp", "Acme/WebApp", " acme/webapp ", "", "İSTANBUL/X", "ΣΑΣ/ΟΔΟΣ", "ẞ/\u212aelvin", "ǅ/Ǆ",
	"\v a/b \u0085", "\u00a0a/b\u3000", "\u200ba/b", "\ta/b\r\n", "\u2028a/b\u2029", "\u180ea/b", "\ufeffa/b",
	"Ω/ſ", "ÀÉÎ/õü", "a/b/c",
}

var repoKeysIn = []string{
	"remote:github.com/acme/webapp", "remote:gitlab.com/acme/webapp", "local:C:/Users/sam/projects/webapp",
	"remote:github.com/acme", "remote:github.com/acme/", "remote:github.com//webapp", "remote:github.com/acme/webapp/extra",
	"Remote:github.com/acme/webapp", "", "remote:github.com/", "remote:github.com/a/b ", "remote:github.com/İ/ẞ",
	" remote:github.com/a/b", "remote:github.com/a//", "remote:github.com/./..",
}

var entryPayloads = []string{
	`{"key":"pr:acme/webapp#1","repo":"acme/webapp","number":1,"url":"u","session":"d/s","role":"author","source":"gh","at":"2026-10-09T12:00:00Z"}`,
	`{"KEY":"k","Repo":"R","NUMBER":2,"URL":"u","SESSION":"s","ROLE":"touched","SOURCE":"x","AT":"2026-10-09T12:00:00.123456789+02:00"}`,
	`{"key":"a","key":"b","Key":"c"}`,
	`{"number":2.0}`,
	`{"number":"2"}`,
	`{"number":-7,"role":"poster"}`,
	`{"number":9223372036854775808}`,
	`{"at":"bad"}`,
	`{"at":null,"url":null,"number":null}`,
	`{"at":"2026-10-09T12:00:00Z","at":"0001-01-01T00:00:00Z"}`,
	`null`,
	`[]`,
	`"x"`,
	``,
	`{"key":"<&>\u2028","extra":{"a":[1,2]}}`,
	"{\"session\":\"d/\xff\xfes\",\"url\":\"\xe2\x82\"}",
	`{"at":"2026-10-09T12:00:00.5-07:30","role":"reviewer"}`,
	`{"at":"2026-10-09t12:00:00z"}`,
	`{"at":"2026-10-09 12:00:00Z"}`,
	`{"repo":"x"} `,
	`{"repo":"x"}{}`,
	`{"ſource":"s","\u212aey":"k"}`,
}

type gEntryOut struct {
	Hex     string `json:"hex"`
	OK      bool   `json:"ok"`
	Marshal string `json:"marshal,omitempty"`
}

type scriptStep struct {
	Op      string   `json:"op"`              // "add", "owners", "forRepo", "forSession"
	Entry   string   `json:"entry,omitempty"` // add: json.Marshal of the entry
	Changed bool     `json:"changed,omitempty"`
	Arg     string   `json:"arg,omitempty"`
	Now     string   `json:"now,omitempty"`
	Out     []string `json:"out,omitempty"` // json.Marshal of each result
}

func marshalString(t *testing.T, v any) string {
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

func TestGoldenLedger(t *testing.T) {
	ledgerGoldenDir(t)

	// Refs.
	type refCase struct {
		Text string       `json:"text"`
		Opts string       `json:"opts"`
		Refs []ledger.Ref `json:"refs"`
	}
	var refs []refCase
	for _, text := range refTexts {
		for _, o := range refOptionSets {
			got := ledger.ExtractRefs(text, ledger.RefOptions{DefaultRepo: o.DefaultRepo, Owner: o.Owner, Aliases: o.Aliases})
			if got == nil {
				got = []ledger.Ref{}
			}
			refs = append(refs, refCase{text, o.Name, got})
		}
	}
	type targetCase struct {
		In  string     `json:"in"`
		OK  bool       `json:"ok"`
		Ref ledger.Ref `json:"ref"`
	}
	var tgs []targetCase
	for _, in := range targets {
		r, ok := ledger.ParseTarget(in)
		tgs = append(tgs, targetCase{in, ok, r})
	}
	writeLedgerGolden(t, "ledger/refs.json", map[string]any{"options": refOptionSets, "extract": refs, "target": tgs})

	// FromBash.
	type bashOut struct {
		Repo   string `json:"repo"`
		Number int    `json:"number"`
		URL    string `json:"url"`
		Role   string `json:"role"`
		Source string `json:"source"`
	}
	type bashGolden struct {
		In  bashCase  `json:"in"`
		Out []bashOut `json:"out"`
	}
	var bash []bashGolden
	for _, in := range bashCases() {
		op, err := hex.DecodeString(in.GitOpHex)
		if err != nil {
			t.Fatal(err)
		}
		if len(op) == 0 {
			op = nil
		}
		out := []bashOut{}
		for _, r := range ledger.FromBash(ledger.BashResult{Command: in.Command, Stdout: in.Stdout, GitOperation: op, DefaultRepo: in.DefaultRepo}) {
			out = append(out, bashOut{r.Ref.Repo, r.Ref.Number, r.URL, string(r.Role), r.Source})
		}
		bash = append(bash, bashGolden{in, out})
	}
	writeLedgerGolden(t, "ledger/bash.json", bash)

	// Keys.
	type keyCase struct {
		Repo    string `json:"repo"`
		N       int    `json:"n"`
		Key     string `json:"key"`
		RepoKey string `json:"repoKey"`
		Back    string `json:"back"`
	}
	var keys []keyCase
	for i, repo := range keyRepos {
		n := []int{1, 1437, 0, -5, 1<<63 - 1}[i%5]
		rk := ledger.RepoKey(repo)
		keys = append(keys, keyCase{repo, n, ledger.Key(repo, n), rk, ledger.RepoFromKey(rk)})
	}
	type fromKey struct {
		In   string `json:"in"`
		Repo string `json:"repo"`
	}
	var fks []fromKey
	for _, k := range repoKeysIn {
		fks = append(fks, fromKey{k, ledger.RepoFromKey(k)})
	}

	// Entry decoding (capture's json.Unmarshal of pr.ledger payloads).
	var decodes []gEntryOut
	for _, p := range entryPayloads {
		var e ledger.Entry
		g := gEntryOut{Hex: hex.EncodeToString([]byte(p))}
		if err := json.Unmarshal([]byte(p), &e); err == nil {
			g.OK = true
			g.Marshal = marshalString(t, e)
		}
		decodes = append(decodes, g)
	}
	writeLedgerGolden(t, "ledger/keys.json", map[string]any{"keys": keys, "fromKey": fks, "decode": decodes})

	// A scripted run of Add and the views.
	rng := rand.New(rand.NewPCG(20261029, 8))
	repos := []string{"acme/webapp", "Acme/WebApp", "acme/storefront", "ACME/STOREFRONT", "other/x"}
	sessions := []string{"dev/a", "dev/b", "mac/c", "dev/a "}
	roles := []ledger.Role{ledger.Author, ledger.Touched, ledger.Reviewer, ledger.Author, ledger.Touched, ledger.Reviewer, "poster", ""}
	sources := []string{"gh", "gitOperation", "pr-link"}
	base := time.Date(2026, 10, 9, 12, 0, 0, 0, time.UTC)
	plus2 := time.FixedZone("", 2*3600)
	minus730 := time.FixedZone("", -(7*3600 + 1800))
	randTime := func() time.Time {
		switch rng.IntN(10) {
		case 0:
			return time.Time{}
		case 1:
			return base.Add(-ledger.Keep).Add(time.Duration(rng.IntN(5)-2) * time.Nanosecond)
		}
		at := base.Add(time.Duration(rng.Int64N(int64(60*24*time.Hour))) - 50*24*time.Hour)
		if rng.IntN(3) == 0 {
			at = at.Truncate(time.Hour)
		}
		switch rng.IntN(3) {
		case 0:
			at = at.In(plus2)
		case 1:
			at = at.In(minus730)
		}
		return at
	}
	nows := []time.Time{base, base.Add(time.Hour), base.Add(-10 * 24 * time.Hour), base.Add(30 * 24 * time.Hour), base.In(plus2)}
	l := ledger.New()
	var steps []scriptStep
	outs := func(es []ledger.Entry) []string {
		out := []string{}
		for _, e := range es {
			out = append(out, marshalString(t, e))
		}
		return out
	}
	for i := 0; i < 400; i++ {
		repo := repos[rng.IntN(len(repos))]
		n := rng.IntN(6) + 1
		switch r := rng.IntN(10); {
		case r < 7:
			e := ledger.Entry{
				Key: ledger.Key(repo, n), Repo: repo, Number: n,
				Session: sessions[rng.IntN(len(sessions))], Role: roles[rng.IntN(len(roles))],
				Source: sources[rng.IntN(len(sources))], At: randTime(),
			}
			switch rng.IntN(6) {
			case 0:
				e.URL = fmt.Sprintf("https://github.com/%s/pull/%d", repo, n)
			case 1:
				e.URL = fmt.Sprintf("https://github.com/%s/pull/%d/files", repo, n)
			case 2:
				e.Key = ""
			case 3:
				if rng.IntN(4) == 0 {
					e.Session = ""
				}
			}
			s := scriptStep{Op: "add", Entry: marshalString(t, e)}
			s.Changed = l.Add(e)
			steps = append(steps, s)
		case r < 8:
			now := nows[rng.IntN(len(nows))]
			k := ledger.Key(repo, n)
			steps = append(steps, scriptStep{Op: "owners", Arg: k, Now: now.Format(time.RFC3339Nano), Out: outs(l.Owners(k, now))})
		case r < 9:
			now := nows[rng.IntN(len(nows))]
			arg := []string{repo, "", " acme/WEBAPP ", "nobody/x"}[rng.IntN(4)]
			steps = append(steps, scriptStep{Op: "forRepo", Arg: arg, Now: now.Format(time.RFC3339Nano), Out: outs(l.ForRepo(arg, now))})
		default:
			now := nows[rng.IntN(len(nows))]
			s := sessions[rng.IntN(len(sessions))]
			steps = append(steps, scriptStep{Op: "forSession", Arg: s, Now: now.Format(time.RFC3339Nano), Out: outs(l.ForSession(s, now))})
		}
	}
	for _, now := range nows {
		steps = append(steps, scriptStep{Op: "forRepo", Arg: "", Now: now.Format(time.RFC3339Nano), Out: outs(l.ForRepo("", now))})
	}
	writeLedgerGolden(t, "ledger/script.json", steps)
}

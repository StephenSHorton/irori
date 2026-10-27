package ledger

import (
	"encoding/json"
	"regexp"
	"strconv"
	"strings"
)

var (
	// prURL finds a GitHub PR URL in output. The host must start the URL
	// (no look-alike such as evilgithub.com) and the number must end at a
	// word boundary (no pull/3abc).
	prURL      = regexp.MustCompile(`(?i)(?:^|[^\w.-])(?:https?://)?(?:www\.)?github\.com/([\w.-]+)/([\w.-]+)/pull/(\d+)\b`)
	ghPrCreate = regexp.MustCompile(`\bgh\s+pr\s+create\b`)
	ghPrReview = regexp.MustCompile(`\bgh\s+pr\s+review(?:\s|$)`)
	digitsOnly = regexp.MustCompile(`^\d+$`)
)

// gitOperation is the part of toolUseResult.gitOperation the ledger reads:
// {"pr":{"number":N,"url":"...","action":"created"}}, maybe beside "push".
type gitOperation struct {
	PR *struct {
		Number int    `json:"number"`
		URL    string `json:"url"`
		Action string `json:"action"`
	} `json:"pr"`
}

func fromBash(b BashResult) []BashRef {
	var out []BashRef
	seen := map[string]bool{}
	push := func(r BashRef) {
		id := key(r.Ref.Repo, r.Ref.Number) + "\x00" + string(r.Role)
		if !seen[id] {
			seen[id] = true
			out = append(out, r)
		}
	}

	if ref, url, ok := createdFromGitOperation(b); ok {
		push(BashRef{Ref: ref, URL: url, Role: Author, Source: "gitOperation"})
	} else if ghPrCreate.MatchString(b.Command) {
		if ref, url, ok := findPRURL(b.Stdout); ok {
			push(BashRef{Ref: ref, URL: url, Role: Author, Source: "gh"})
		}
	}

	// Each review's arguments run to the first unquoted separator; the next
	// search starts after them, so "gh pr review" quoted in a body is text.
	for cmd := b.Command; ; {
		loc := ghPrReview.FindStringIndex(cmd)
		if loc == nil {
			break
		}
		args, n := shellWords(cmd[loc[1]:])
		if ref, url, ok := reviewTarget(args, b.DefaultRepo); ok {
			push(BashRef{Ref: ref, URL: url, Role: Reviewer, Source: "gh"})
		}
		cmd = cmd[loc[1]+n:]
	}
	return out
}

// findPRURL reads the first GitHub PR URL in s, as a Ref and its canonical
// https://github.com/<o>/<r>/pull/<n> form.
func findPRURL(s string) (Ref, string, bool) {
	m := prURL.FindStringSubmatch(s)
	if m == nil || !validSegment(m[1]) || !validSegment(m[2]) {
		return Ref{}, "", false
	}
	n, ok := prNumber(m[3])
	if !ok {
		return Ref{}, "", false
	}
	return prRef(m[1]+"/"+m[2], n)
}

func prRef(repo string, n int) (Ref, string, bool) {
	return Ref{Repo: repo, Number: n}, "https://github.com/" + repo + "/pull/" + strconv.Itoa(n), true
}

// createdFromGitOperation reads gitOperation.pr when its action is "created".
// The repo comes from the URL, else DefaultRepo; a URL that is not a GitHub
// PR is dropped.
func createdFromGitOperation(b BashResult) (Ref, string, bool) {
	if len(b.GitOperation) == 0 {
		return Ref{}, "", false
	}
	var op gitOperation
	if json.Unmarshal(b.GitOperation, &op) != nil || op.PR == nil || op.PR.Action != "created" {
		return Ref{}, "", false
	}
	n, repo, url := op.PR.Number, b.DefaultRepo, ""
	if ref, u, ok := findPRURL(op.PR.URL); ok {
		repo, url = ref.Repo, u
		if n <= 0 {
			n = ref.Number
		}
	}
	if repo == "" || n <= 0 {
		return Ref{}, "", false
	}
	return Ref{Repo: repo, Number: n}, url, true
}

// shellWords splits s like a shell into words, honouring '...' and "..."
// (with \" and \ escapes inside double quotes), up to the first unquoted
// ; & | or line break. Outside quotes a backslash is literal, so Windows
// paths survive. It returns the words and the bytes consumed.
func shellWords(s string) ([]string, int) {
	var words []string
	var w strings.Builder
	inWord := false
	var quote byte
	i := 0
	for ; i < len(s); i++ {
		c := s[i]
		switch {
		case quote == '\'':
			if c == '\'' {
				quote = 0
			} else {
				w.WriteByte(c)
			}
		case quote == '"':
			if c == '"' {
				quote = 0
			} else if c == '\\' && i+1 < len(s) && (s[i+1] == '"' || s[i+1] == '\\') {
				i++
				w.WriteByte(s[i])
			} else {
				w.WriteByte(c)
			}
		case c == '\'' || c == '"':
			quote, inWord = c, true
		case c == ';' || c == '&' || c == '|' || c == '\n' || c == '\r':
			if inWord {
				words = append(words, w.String())
			}
			return words, i
		case c == ' ' || c == '\t':
			if inWord {
				words = append(words, w.String())
				w.Reset()
				inWord = false
			}
		default:
			w.WriteByte(c)
			inWord = true
		}
	}
	if inWord {
		words = append(words, w.String())
	}
	return words, i
}

// reviewValueFlags are gh pr review's flags that take a value; its other
// flags (-a, -c, -r, -h and their long forms) take none.
var reviewValueFlags = map[string]bool{"-b": true, "--body": true, "-F": true, "--body-file": true, "-R": true, "--repo": true}

// reviewTarget reads the words after "gh pr review". The target is the first
// positional word; flags are skipped with their values, and "--" ends them.
// A URL names its PR; a number takes the repo from -R/--repo (the last one;
// [HOST/]OWNER/REPO, host github.com only), else defaultRepo. A -R value
// that is not a GitHub repo skips the review.
func reviewTarget(args []string, defaultRepo string) (Ref, string, bool) {
	target, repoFlag, hasRepo := "", "", false
	flags := true
	for i := 0; i < len(args); i++ {
		f := args[i]
		if !flags || f == "-" || !strings.HasPrefix(f, "-") {
			if target == "" {
				target = f
			}
			continue
		}
		switch {
		case f == "--":
			flags = false
		case reviewValueFlags[f]:
			if i+1 < len(args) {
				i++
				if f == "-R" || f == "--repo" {
					repoFlag, hasRepo = args[i], true
				}
			}
		case strings.HasPrefix(f, "--"):
			if name, v, ok := strings.Cut(f, "="); ok && name == "--repo" {
				repoFlag, hasRepo = v, true
			}
		case strings.HasPrefix(f, "-R"):
			repoFlag, hasRepo = f[2:], true
		}
	}
	if target == "" {
		return Ref{}, "", false
	}
	if strings.Contains(target, "/") {
		m := targetURL.FindStringSubmatch(target)
		if m == nil || !validSegment(m[1]) || !validSegment(m[2]) {
			return Ref{}, "", false
		}
		n, ok := prNumber(m[3])
		if !ok {
			return Ref{}, "", false
		}
		return prRef(m[1]+"/"+m[2], n)
	}
	if !digitsOnly.MatchString(target) {
		return Ref{}, "", false
	}
	n, ok := prNumber(target)
	if !ok {
		return Ref{}, "", false
	}
	repo := defaultRepo
	if hasRepo {
		if repo, ok = repoArg(repoFlag); !ok {
			return Ref{}, "", false
		}
	}
	if repo == "" {
		return Ref{}, "", false
	}
	return Ref{Repo: repo, Number: n}, "", true
}

// repoArg reads a -R/--repo value, [HOST/]OWNER/REPO, as "owner/name". Only
// github.com (or www.github.com) is accepted as a host: the ledger keys are
// github.com repos.
func repoArg(v string) (string, bool) {
	parts := strings.Split(strings.TrimSuffix(v, "/"), "/")
	if len(parts) == 3 {
		h := strings.ToLower(parts[0])
		if h != "github.com" && h != "www.github.com" {
			return "", false
		}
		parts = parts[1:]
	}
	if len(parts) != 2 || !validSegment(parts[0]) || !validSegment(parts[1]) {
		return "", false
	}
	return parts[0] + "/" + parts[1], true
}

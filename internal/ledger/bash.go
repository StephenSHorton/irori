package ledger

import (
	"encoding/json"
	"regexp"
	"strings"
)

var (
	prURL      = regexp.MustCompile(`github\.com/([\w.-]+/[\w.-]+)/pull/(\d+)`)
	ghPrCreate = regexp.MustCompile(`\bgh\s+pr\s+create\b`)
	ghPrReview = regexp.MustCompile(`\bgh\s+pr\s+review\s+([^\n;&|]*)`)
	repoFlag   = regexp.MustCompile(`(?:-R|--repo)[\s=]+["']?([\w.-]+/[\w.-]+)`)
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
		if m := prURL.FindStringSubmatch(b.Stdout); m != nil {
			if n, ok := prNumber(m[2]); ok {
				push(BashRef{Ref: Ref{Repo: m[1], Number: n}, URL: "https://" + m[0], Role: Author, Source: "gh"})
			}
		}
	}

	for _, m := range ghPrReview.FindAllStringSubmatch(b.Command, -1) {
		if ref, url, ok := reviewTarget(m[1], b.DefaultRepo); ok {
			push(BashRef{Ref: ref, URL: url, Role: Reviewer, Source: "gh"})
		}
	}
	return out
}

// createdFromGitOperation reads gitOperation.pr when its action is "created".
// The repo comes from the URL, else DefaultRepo.
func createdFromGitOperation(b BashResult) (Ref, string, bool) {
	if len(b.GitOperation) == 0 {
		return Ref{}, "", false
	}
	var op gitOperation
	if json.Unmarshal(b.GitOperation, &op) != nil || op.PR == nil || op.PR.Action != "created" {
		return Ref{}, "", false
	}
	n, repo := op.PR.Number, b.DefaultRepo
	if m := prURL.FindStringSubmatch(op.PR.URL); m != nil {
		repo = m[1]
		if n <= 0 {
			n, _ = prNumber(m[2])
		}
	}
	if repo == "" || n <= 0 {
		return Ref{}, "", false
	}
	return Ref{Repo: repo, Number: n}, op.PR.URL, true
}

// reviewTarget reads the arguments after "gh pr review". The target is the
// first token (quotes stripped), skipping leading -R/--repo flags, which
// tsukumo did not. A URL names its PR; a number takes the repo from
// -R/--repo anywhere in args, else defaultRepo.
func reviewTarget(args, defaultRepo string) (Ref, string, bool) {
	fields := strings.Fields(args)
	target := ""
	for i := 0; i < len(fields); i++ {
		f := fields[i]
		if f == "-R" || f == "--repo" {
			i++
			continue
		}
		if strings.HasPrefix(f, "--repo=") {
			continue
		}
		target = strings.Trim(f, `"'`)
		break
	}
	if m := prURL.FindStringSubmatch(target); m != nil {
		n, ok := prNumber(m[2])
		if !ok {
			return Ref{}, "", false
		}
		url := target
		if !strings.Contains(url, "://") {
			url = "https://" + url
		}
		return Ref{Repo: m[1], Number: n}, url, true
	}
	if !digitsOnly.MatchString(target) {
		return Ref{}, "", false
	}
	n, ok := prNumber(target)
	if !ok {
		return Ref{}, "", false
	}
	repo := defaultRepo
	if m := repoFlag.FindStringSubmatch(args); m != nil {
		repo = m[1]
	}
	if repo == "" {
		return Ref{}, "", false
	}
	return Ref{Repo: repo, Number: n}, "", true
}

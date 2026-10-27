package coord

import (
	"fmt"
	"net/url"
	"path"
	"regexp"
	"strings"
	"unicode/utf8"
)

// LinkKind says what a card's link points at.
type LinkKind string

const (
	LinkURL    LinkKind = "url"    // http(s) URL: Linear, Notion, a GitHub issue or PR, anything
	LinkPath   LinkKind = "path"   // a repo path, forward slashes
	LinkCommit LinkKind = "commit" // a commit sha (7-40 hex, lowercased)
)

// Link is one link out from a card (board.md "Links out"). Irori only
// points: it never syncs or mirrors what a link names.
type Link struct {
	Kind  LinkKind `json:"kind"`
	Ref   string   `json:"ref"`   // the URL, path or sha
	Label string   `json:"label"` // LinkLabel(Ref), what the chip shows
}

// Link limits.
const (
	MaxLinks   = 16
	maxLinkRef = 1000
	maxLabel   = 60
)

var (
	hexRe         = regexp.MustCompile(`^[0-9a-fA-F]{7,40}$`)
	linearIssueRe = regexp.MustCompile(`^/[^/]+/issue/([A-Za-z][A-Za-z0-9]*-[0-9]+)(?:/|$)`)
	githubItemRe  = regexp.MustCompile(`^/[^/]+/[^/]+/(?:issues|pull)/([0-9]+)(?:/|$)`)
	notionIDRe    = regexp.MustCompile(`-?[0-9a-f]{32}$`)
)

// ParseLink classifies s as a URL (it has a scheme, which must be http or
// https), a commit sha (7-40 hex) or a repo path (anything else without a
// scheme; backslashes become slashes, no ".." segments), and labels it.
func ParseLink(s string) (Link, error) {
	s = strings.TrimSpace(s)
	if s == "" || utf8.RuneCountInString(s) > maxLinkRef || strings.ContainsAny(s, "\r\n\t") {
		return Link{}, fmt.Errorf("%w: a link is a URL, a repo path or a commit sha (at most %d characters)", ErrInvalid, maxLinkRef)
	}
	if strings.Contains(s, "://") {
		u, err := url.Parse(s)
		if err != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Host == "" {
			return Link{}, fmt.Errorf("%w: link %q is not an http(s) URL", ErrInvalid, s)
		}
		return Link{Kind: LinkURL, Ref: s, Label: LinkLabel(s)}, nil
	}
	if hexRe.MatchString(s) {
		s = strings.ToLower(s)
		return Link{Kind: LinkCommit, Ref: s, Label: LinkLabel(s)}, nil
	}
	p := strings.ReplaceAll(s, `\`, "/")
	for _, seg := range strings.Split(p, "/") {
		if seg == ".." {
			return Link{}, fmt.Errorf("%w: link path %q leaves the repo", ErrInvalid, s)
		}
	}
	return Link{Kind: LinkPath, Ref: p, Label: LinkLabel(p)}, nil
}

// LinkLabel is the name a card shows for a link ref:
//
//	https://linear.app/<ws>/issue/ABC-123/...  -> "Linear ABC-123"
//	https://github.com/<o>/<r>/issues/57       -> "GitHub #57" (pull/57 too)
//	https://www.notion.so/<ws>/Board-design-<32 hex> -> "Notion Board design" ("Notion" without a title)
//	any other URL                               -> its host, without "www."
//	a commit sha                                -> "commit <first 7>"
//	a path                                      -> its last two segments
//
// Labels are at most 60 runes.
func LinkLabel(ref string) string {
	ref = strings.TrimSpace(ref)
	if hexRe.MatchString(ref) {
		return "commit " + strings.ToLower(ref[:7])
	}
	if !strings.Contains(ref, "://") {
		p := strings.Trim(strings.ReplaceAll(ref, `\`, "/"), "/")
		segs := strings.Split(p, "/")
		if len(segs) > 2 {
			p = ".../" + strings.Join(segs[len(segs)-2:], "/")
		}
		return capLabel(p)
	}
	u, err := url.Parse(ref)
	if err != nil || u.Host == "" {
		return capLabel(ref)
	}
	host := strings.TrimPrefix(strings.ToLower(u.Hostname()), "www.")
	switch {
	case host == "linear.app":
		if m := linearIssueRe.FindStringSubmatch(u.Path); m != nil {
			return "Linear " + strings.ToUpper(m[1])
		}
		return "Linear"
	case host == "github.com":
		if m := githubItemRe.FindStringSubmatch(u.Path); m != nil {
			return "GitHub #" + m[1]
		}
		return "GitHub"
	case host == "notion.so" || host == "notion.com" || strings.HasSuffix(host, ".notion.site"):
		last := path.Base(strings.TrimSuffix(u.Path, "/"))
		title := strings.TrimSpace(strings.ReplaceAll(notionIDRe.ReplaceAllString(last, ""), "-", " "))
		if title == "" || title == "." || title == "/" {
			return "Notion"
		}
		return capLabel("Notion " + title)
	}
	return capLabel(host)
}

func capLabel(s string) string {
	if utf8.RuneCountInString(s) <= maxLabel {
		return s
	}
	r := []rune(s)
	return string(r[:maxLabel-1]) + "…"
}

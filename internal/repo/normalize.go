package repo

import (
	"net/url"
	"strings"
	"unicode"
)

// remoteSchemes are the URL schemes that name a hosted repository. file://
// and anything else is local or unknown and yields "".
var remoteSchemes = map[string]bool{
	"http": true, "https": true, "ssh": true, "git": true,
	"git+ssh": true, "ssh+git": true, "git+https": true,
}

func normalizeRemote(raw string) string {
	s := strings.TrimSpace(raw)
	if s == "" || isLocalPath(s) {
		return ""
	}

	var host, path string
	if strings.Contains(s, "://") {
		u, err := url.Parse(s)
		if err != nil || !remoteSchemes[strings.ToLower(u.Scheme)] {
			return ""
		}
		// u.Path is already percent-decoded, so %2F would pass as a separator.
		// Decode each escaped segment and refuse one that holds a slash.
		host = u.Hostname()
		raw := strings.Split(u.EscapedPath(), "/")
		for i, seg := range raw {
			dec, err := url.PathUnescape(seg)
			if err != nil || strings.ContainsAny(dec, "/\\") {
				return ""
			}
			raw[i] = dec
		}
		path = strings.Join(raw, "/")
	} else {
		// scp-like: [user@]host:path. The colon must come before any slash,
		// or it's a relative path such as "dir/a:b".
		i := strings.IndexByte(s, ':')
		if i < 0 {
			return ""
		}
		if j := strings.IndexByte(s, '/'); j >= 0 && j < i {
			return ""
		}
		host, path = s[:i], s[i+1:]
		if at := strings.LastIndexByte(host, '@'); at >= 0 {
			host = host[at+1:]
		}
		// A one-letter "host" is a Windows drive (C:foo).
		if len(host) <= 1 {
			return ""
		}
	}

	host = strings.TrimSuffix(strings.ToLower(host), ".")
	if host == "" || strings.ContainsAny(host, "/\\ ") {
		return ""
	}
	segs := splitPath(strings.ToLower(path))
	if segs == nil {
		return ""
	}
	host, segs = foldAzure(host, segs)
	if len(segs) < 2 {
		return ""
	}
	return host + "/" + strings.Join(segs, "/")
}

// splitPath drops empty segments and a trailing .git. It returns nil for a
// path with "." or ".." segments or with control characters.
func splitPath(p string) []string {
	var segs []string
	for _, seg := range strings.Split(p, "/") {
		switch seg {
		case "":
			continue
		case ".", "..":
			return nil
		}
		if strings.ContainsFunc(seg, unicode.IsControl) {
			return nil
		}
		segs = append(segs, seg)
	}
	if n := len(segs); n > 0 {
		last := strings.TrimSuffix(segs[n-1], ".git")
		if last == "" {
			segs = segs[:n-1]
		} else {
			segs[n-1] = last
		}
	}
	if len(segs) == 0 {
		return nil
	}
	return segs
}

// foldAzure maps every Azure DevOps spelling of a repo onto the https shape
// dev.azure.com/<org>/<project>/_git/<repo>, so ssh and https clones match.
func foldAzure(host string, segs []string) (string, []string) {
	host, segs = foldAzureHost(host, segs)
	// Legacy ssh clones spell the _git segment _ssh:
	// ssh://org@org.visualstudio.com:22/DefaultCollection/proj/_ssh/repo
	if n := len(segs); host == "dev.azure.com" && n >= 2 && segs[n-2] == "_ssh" {
		segs[n-2] = "_git"
	}
	return host, segs
}

func foldAzureHost(host string, segs []string) (string, []string) {
	switch {
	case host == "ssh.dev.azure.com" || host == "vs-ssh.visualstudio.com":
		// git@ssh.dev.azure.com:v3/org/project/repo
		if len(segs) == 4 && segs[0] == "v3" {
			return "dev.azure.com", []string{segs[1], segs[2], "_git", segs[3]}
		}
	case strings.HasSuffix(host, ".visualstudio.com"):
		// https://org.visualstudio.com[/DefaultCollection]/project/_git/repo
		org := strings.TrimSuffix(host, ".visualstudio.com")
		if len(segs) > 0 && segs[0] == "defaultcollection" {
			segs = segs[1:]
		}
		return "dev.azure.com", append([]string{org}, segs...)
	}
	return host, segs
}

// isLocalPath reports paths git accepts as a local remote: absolute, relative,
// home-relative, Windows drive and UNC paths.
func isLocalPath(s string) bool {
	if strings.HasPrefix(strings.ToLower(s), "file:") {
		return true
	}
	switch {
	case strings.HasPrefix(s, "/"), strings.HasPrefix(s, "\\"),
		strings.HasPrefix(s, "."), strings.HasPrefix(s, "~"):
		return true
	}
	if len(s) >= 2 && s[1] == ':' && isLetter(s[0]) {
		return len(s) == 2 || s[2] == '\\' || s[2] == '/'
	}
	return false
}

func isLetter(c byte) bool { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') }

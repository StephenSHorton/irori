package scrub

import "regexp"

// Strict is the stricter pass Share to board runs after String (design
// v0.5-shared-boards.md §5.2): it drops absolute paths and home
// directories from text that is about to reach other people, since a path
// says whose machine and which folders. Each one becomes "[path]". Relative
// paths (src/x.go, ./build) and URLs stay: a shared card may name files in
// its repo, and links are what git already makes public.
func Strict(in string) string {
	out := in
	for _, re := range strictPaths {
		out = re.ReplaceAllString(out, "${1}"+PathMarker)
	}
	return out
}

// PathMarker is what Strict puts where a path was.
const PathMarker = "[path]"

// StrictDropped reports whether Strict would change in.
func StrictDropped(in string) bool { return Strict(in) != in }

// strictPaths match, each with the text before the path kept as $1:
//   - a file:// URL, whatever its host: it names a file on someone's disk;
//   - a Windows drive path, C:\Users\me or c:/work;
//   - a UNC path, \\server\share;
//   - a home directory, ~/x, ~\x, $HOME/x, %USERPROFILE%\x;
//   - a POSIX absolute path of two or more parts, /Users/me or /home/me/x,
//     only where a path can start (pathStart: line start, space, quote, a
//     markdown code span or emphasis, bracket, '=', ',', ':'), so a URL's
//     "//host/path" and "and/or" stay.
//
// A path ends at whitespace, a quote, '`', ')' or ']' (pathRest), so
// "(/Users/me/x)" keeps its closing paren and a code span its tick.
var strictPaths = []*regexp.Regexp{
	regexp.MustCompile(`(?i)(^|[^A-Za-z0-9])file://` + pathRest),
	regexp.MustCompile(`(^|[^A-Za-z0-9])[A-Za-z]:[\\/]` + pathRest),
	regexp.MustCompile(`(^|` + pathStart + `)\\\\[^\s"'<>|*?\\)\]` + "`" + `]+(?:\\` + pathRest + `)*`),
	regexp.MustCompile(`(^|` + pathStart + `)(?:~|\$HOME|\$\{HOME\}|%USERPROFILE%|%HOMEPATH%)(?:[\\/]` + pathRest + `)?`),
	regexp.MustCompile(`(^|` + pathStart + `)/[A-Za-z0-9._@+-]+/` + pathRest),
}

// pathStart is what may come just before a path that begins with '/', '~',
// '$', '%' or '\'; pathRest is the rest of a path, up to where it ends.
const (
	pathStart = `[\s"'(\[=,:*` + "`" + `]`
	pathRest  = `[^\s"'<>|*?)\]` + "`" + `]*`
)

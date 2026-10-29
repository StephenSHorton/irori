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
//   - a Windows drive path, C:\Users\me or c:/work;
//   - a UNC path, \\server\share;
//   - a home directory, ~/x, ~\x, $HOME/x, %USERPROFILE%\x;
//   - a POSIX absolute path of two or more parts, /Users/me or /home/me/x,
//     only where a path can start (line start, space, quote, bracket, '=',
//     ','), so a URL's "//host/path" and "and/or" stay.
var strictPaths = []*regexp.Regexp{
	regexp.MustCompile(`(^|[^A-Za-z0-9])[A-Za-z]:[\\/][^\s"'<>|*?]*`),
	regexp.MustCompile(`(^|[\s"'(\[=,])\\\\[^\s"'<>|*?\\]+(?:\\[^\s"'<>|*?\\]*)*`),
	regexp.MustCompile(`(^|[\s"'(\[=,])(?:~|\$HOME|\$\{HOME\}|%USERPROFILE%|%HOMEPATH%)(?:[\\/][^\s"'<>|*?]*)?`),
	regexp.MustCompile(`(^|[\s"'(\[=,])/[A-Za-z0-9._@+-]+/[^\s"'<>|*?]*`),
}

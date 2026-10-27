package gitlink

import (
	"path"
	"strings"
)

// commitSubcommands are the git subcommands that can move HEAD onto new
// commits. Plumbing such as `git commit-tree` is left out on purpose: it only
// writes an object, and the commit reaches the reflog later through
// update-ref or a porcelain command, which is what gets matched.
var commitSubcommands = map[string]bool{
	"commit": true, "merge": true, "cherry-pick": true, "revert": true,
	"rebase": true, "am": true, "pull": true,
}

// gitArgOptions are git global options that take their value as the next
// word.
var gitArgOptions = map[string]bool{
	"-C": true, "-c": true, "--git-dir": true, "--work-tree": true,
	"--namespace": true, "--super-prefix": true, "--config-env": true,
}

// wrappers are words that run the rest of the line as a command: programs
// that take one, and the shell keywords a command can follow.
var wrappers = map[string]bool{
	"sudo": true, "env": true, "command": true, "time": true, "nohup": true,
	"exec": true, "xargs": true, "builtin": true, "timeout": true, "nice": true,
	"{": true, "}": true, "!": true, "if": true, "then": true, "else": true,
	"elif": true, "do": true, "while": true, "until": true,
}

// wrapperValues are the flags of a wrapper that take the next word as their
// value, and wrapperPositional how many leading non-flag words it takes
// before the command (timeout's duration).
var wrapperValues = map[string]map[string]bool{
	"sudo":    {"-u": true, "-g": true, "-C": true, "-D": true, "-h": true, "-p": true, "-r": true, "-t": true, "-U": true, "-T": true, "--user": true, "--group": true, "--chdir": true, "--host": true, "--prompt": true, "--role": true, "--type": true, "--other-user": true, "--close-from": true, "--command-timeout": true},
	"env":     {"-u": true, "-C": true, "--unset": true, "--chdir": true},
	"nice":    {"-n": true, "--adjustment": true},
	"timeout": {"-s": true, "-k": true, "--signal": true, "--kill-after": true},
	"xargs":   {"-a": true, "-d": true, "-E": true, "-I": true, "-L": true, "-n": true, "-P": true, "-s": true, "--arg-file": true, "--delimiter": true, "--max-args": true, "--max-procs": true, "--max-chars": true},
	"time":    {"-f": true, "-o": true, "--format": true, "--output": true},
}

var wrapperPositional = map[string]int{"timeout": 1}

// shells run their -c argument as a command line.
var shells = map[string]bool{"bash": true, "sh": true, "zsh": true, "dash": true}

// isCommitCommand splits command into simple commands with a small shell
// lexer (quotes, backslashes, comments, and the separators && || ; | & ( )
// and newlines) and checks each one. Text inside quotes is never a command
// of its own, except the argument of `bash -c` / `sh -c` and `eval`, which
// is parsed again. Heredoc bodies are not recognized, so a line in one that
// reads like a commit command counts: a false positive only widens the
// committer search, which is acceptable.
func isCommitCommand(command string) bool {
	return checkLine(command, 0)
}

func checkLine(line string, depth int) bool {
	if depth > 4 {
		return false
	}
	for _, words := range splitCommands(line) {
		if checkWords(words, depth) {
			return true
		}
	}
	return false
}

func checkWords(w []string, depth int) bool {
	// Skip env assignments and wrappers, with their flags and values.
	for len(w) > 0 {
		switch name := progName(w[0]); {
		case isAssignment(w[0]):
			w = w[1:]
		case wrappers[name]:
			w = skipWrapperArgs(name, w[1:])
		default:
			goto run
		}
	}
	return false
run:
	switch prog := progName(w[0]); {
	case prog == "git":
		return isGitCommit(w[1:])
	case prog == "gh":
		return isGhMerge(w[1:])
	case prog == "eval":
		return checkLine(strings.Join(w[1:], " "), depth+1)
	case shells[prog]:
		for i := 1; i < len(w); i++ {
			if strings.HasPrefix(w[i], "-") && !strings.HasPrefix(w[i], "--") && strings.Contains(w[i], "c") && i+1 < len(w) {
				return checkLine(w[i+1], depth+1)
			}
		}
	}
	return false
}

// skipWrapperArgs drops the flags of wrapper name (and the values of those
// in wrapperValues), a "--", and its leading positional words.
func skipWrapperArgs(name string, w []string) []string {
	for len(w) > 0 && strings.HasPrefix(w[0], "-") {
		opt := w[0]
		w = w[1:]
		if opt == "--" {
			break
		}
		if wrapperValues[name][opt] && len(w) > 0 {
			w = w[1:]
		}
	}
	for n := wrapperPositional[name]; n > 0 && len(w) > 0; n-- {
		w = w[1:]
	}
	return w
}

// commitValueOptions are options of the commit-making subcommands whose
// value is the next word, so a message like "--abort" is not read as one.
var commitValueOptions = map[string]bool{
	"-m": true, "-F": true, "-C": true, "-c": true, "-t": true, "-s": true,
	"-X": true, "-x": true, "--message": true, "--file": true,
	"--template": true, "--author": true, "--date": true, "--strategy": true,
	"--strategy-option": true, "--onto": true, "--exec": true,
	"--reuse-message": true, "--reedit-message": true, "--fixup": true,
	"--squash": true, "--cleanup": true, "--mainline": true,
}

func isGitCommit(w []string) bool {
	for len(w) > 0 && strings.HasPrefix(w[0], "-") {
		opt := w[0]
		w = w[1:]
		if gitArgOptions[opt] && len(w) > 0 {
			w = w[1:]
		}
	}
	if len(w) == 0 || !commitSubcommands[w[0]] {
		return false
	}
	args := w[1:]
	for i := 0; i < len(args); i++ {
		switch a := args[i]; {
		case a == "--":
			return true
		case a == "--abort" || a == "--quit" || a == "-h" || a == "--help":
			return false
		case commitValueOptions[a]:
			i++
		}
	}
	return true
}

func isGhMerge(w []string) bool {
	var pos []string
	for i := 0; i < len(w) && len(pos) < 2; i++ {
		a := w[i]
		switch {
		case a == "-R" || a == "--repo":
			i++
		case strings.HasPrefix(a, "-"):
		default:
			pos = append(pos, a)
		}
	}
	return len(pos) == 2 && pos[0] == "pr" && pos[1] == "merge"
}

// progName is a command word's program name: its base name, lowercased,
// without ".exe", so "/usr/bin/git" and `C:\Program Files\Git\cmd\git.exe`
// both read "git".
func progName(word string) string {
	word = strings.ReplaceAll(word, `\`, "/")
	name := strings.ToLower(path.Base(word))
	return strings.TrimSuffix(name, ".exe")
}

func isAssignment(word string) bool {
	eq := strings.IndexByte(word, '=')
	if eq <= 0 {
		return false
	}
	for i := 0; i < eq; i++ {
		c := word[i]
		if !(c == '_' || isLetter(c) || (i > 0 && '0' <= c && c <= '9')) {
			return false
		}
	}
	return true
}

// splitCommands lexes line into simple commands, each a list of words with
// quotes removed.
func splitCommands(line string) [][]string {
	var cmds [][]string
	var words []string
	var cur strings.Builder
	inWord := false
	endWord := func() {
		if inWord {
			words = append(words, cur.String())
			cur.Reset()
			inWord = false
		}
	}
	endCmd := func() {
		endWord()
		if len(words) > 0 {
			cmds = append(cmds, words)
			words = nil
		}
	}
	for i := 0; i < len(line); i++ {
		c := line[i]
		switch {
		case c == '\'':
			inWord = true
			j := strings.IndexByte(line[i+1:], '\'')
			if j < 0 {
				cur.WriteString(line[i+1:])
				i = len(line)
			} else {
				cur.WriteString(line[i+1 : i+1+j])
				i += j + 1
			}
		case c == '"':
			inWord = true
			for i++; i < len(line) && line[i] != '"'; i++ {
				if line[i] == '\\' && i+1 < len(line) && strings.IndexByte("\"\\$`\n", line[i+1]) >= 0 {
					i++
					if line[i] == '\n' {
						continue
					}
				}
				cur.WriteByte(line[i])
			}
		case c == '\\':
			if i+1 < len(line) {
				i++
				if line[i] == '\n' || (line[i] == '\r' && i+1 < len(line) && line[i+1] == '\n') {
					if line[i] == '\r' {
						i++
					}
					continue
				}
				inWord = true
				cur.WriteByte(line[i])
			}
		case c == '#' && !inWord:
			for i < len(line) && line[i] != '\n' {
				i++
			}
			endCmd()
		case c == ' ' || c == '\t' || c == '\r':
			endWord()
		case strings.IndexByte(";&|()\n`", c) >= 0:
			endCmd()
		case c == '$' && i+1 < len(line) && line[i+1] == '(':
			endCmd()
			i++
		default:
			inWord = true
			cur.WriteByte(c)
		}
	}
	endCmd()
	return cmds
}

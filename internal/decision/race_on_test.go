//go:build race

package decision

// slow scales time bounds in tests: the race detector makes code several
// times slower (CI runs macOS with -race).
const slow = 10

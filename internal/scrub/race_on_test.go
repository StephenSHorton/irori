//go:build race

package scrub

// slow scales time bounds in tests: the race detector makes code several
// times slower (CI runs macOS with -race).
const slow = 10

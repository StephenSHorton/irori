//go:build !race

package scrub

// slow scales time bounds in tests; see race_on_test.go.
const slow = 1

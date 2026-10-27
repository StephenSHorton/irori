//go:build !race

package notify

// slow scales time bounds in tests; see race_on_test.go.
const slow = 1

package blob

import (
	"errors"
	"syscall"
)

// Windows error codes an open can hit while another handle renames or
// deletes the file.
const (
	errorAccessDenied     syscall.Errno = 5
	errorSharingViolation syscall.Errno = 32
)

// transientOpen reports whether an open failed only because another process
// is placing or replacing the file at that moment.
func transientOpen(err error) bool {
	var errno syscall.Errno
	if !errors.As(err, &errno) {
		return false
	}
	return errno == errorSharingViolation || errno == errorAccessDenied
}

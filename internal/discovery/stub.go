package discovery

import "errors"

// Skeleton stubs. The discovery/api work package replaces this file.

func newToken() (string, error)     { return "", errors.New("unimplemented") }
func write(string, Lockfile) error  { return errors.New("unimplemented") }
func read(string) (Lockfile, error) { return Lockfile{}, ErrNotRunning }
func remove(string, int) error      { return nil }

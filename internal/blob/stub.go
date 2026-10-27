package blob

import "errors"

// stub: replaced by the blob work package.
var errUnimplemented = errors.New("blob: not implemented yet")

type store struct{}

func open(dir string) (*Store, error)            { return nil, errUnimplemented }
func (s *store) put(data []byte) (string, error) { return "", errUnimplemented }
func (s *store) get(id string) ([]byte, error)   { return nil, errUnimplemented }
func (s *store) has(id string) bool              { return false }

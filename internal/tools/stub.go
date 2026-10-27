package tools

import (
	"context"
	"encoding/json"
)

// Skeleton stubs. The tools work package replaces this file.

type set struct{}

func newSet(Backend) *Set          { return &Set{impl: &set{}} }
func (s *Set) specs(string) []Spec { return nil }
func (s *Set) call(context.Context, Caller, string, json.RawMessage) (string, error) {
	return "", ErrUnknownTool
}

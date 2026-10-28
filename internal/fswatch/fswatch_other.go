//go:build !windows && !(darwin && cgo)

package fswatch

import (
	"context"
	"time"
)

func watch(ctx context.Context, root string, recursive bool, debounce time.Duration) (*Watcher, error) {
	return nil, ErrUnsupported
}

package responses

import (
	"context"
	"errors"
	"fmt"
	"testing"
)

func TestIsClientDisconnectError(t *testing.T) {
	tests := []struct {
		name     string
		err      error
		expected bool
	}{
		{name: "broken pipe", err: errors.New("write tcp: broken pipe"), expected: true},
		{name: "connection reset", err: errors.New("read tcp: connection reset by peer"), expected: true},
		{name: "context canceled", err: context.Canceled, expected: true},
		{name: "wrapped context canceled", err: fmt.Errorf("upstream read: %w", context.Canceled), expected: true},
		{name: "unexpected eof", err: errors.New("unexpected EOF"), expected: false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := isClientDisconnectError(tt.err); got != tt.expected {
				t.Errorf("isClientDisconnectError(%v) = %v, want %v", tt.err, got, tt.expected)
			}
		})
	}
}

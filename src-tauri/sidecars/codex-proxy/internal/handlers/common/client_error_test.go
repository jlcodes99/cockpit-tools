package common

import (
	"context"
	"errors"
	"fmt"
	"testing"
)

func TestIsClientSideError(t *testing.T) {
	tests := []struct {
		name     string
		err      error
		expected bool
	}{
		{
			name:     "nil error",
			err:      nil,
			expected: false,
		},
		{
			name:     "context.Canceled",
			err:      context.Canceled,
			expected: true,
		},
		{
			name:     "wrapped context.Canceled",
			err:      fmt.Errorf("request failed: %w", context.Canceled),
			expected: true,
		},
		{
			name:     "deeply wrapped context.Canceled",
			err:      fmt.Errorf("outer: %w", fmt.Errorf("inner: %w", context.Canceled)),
			expected: true,
		},
		{
			name:     "context.DeadlineExceeded - not client cancel",
			err:      context.DeadlineExceeded,
			expected: false, // 可能是服务端超时
		},
		{
			name:     "broken pipe - connection issue, should failover",
			err:      errors.New("write tcp: broken pipe"),
			expected: false, // 连接故障，应继续 failover
		},
		{
			name:     "connection reset - connection issue, should failover",
			err:      errors.New("read tcp: connection reset by peer"),
			expected: false, // 连接故障，应继续 failover
		},
		{
			name:     "EOF - upstream issue",
			err:      errors.New("unexpected EOF"),
			expected: false,
		},
		{
			name:     "normal error",
			err:      errors.New("upstream error: 500"),
			expected: false,
		},
		{
			name:     "network timeout",
			err:      errors.New("dial tcp: i/o timeout"),
			expected: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := isClientSideError(tt.err)
			if result != tt.expected {
				t.Errorf("isClientSideError(%v) = %v, expected %v", tt.err, result, tt.expected)
			}
		})
	}
}

func TestIsClientDisconnectError(t *testing.T) {
	tests := []struct {
		name     string
		err      error
		expected bool
	}{
		{name: "broken pipe", err: errors.New("write tcp: broken pipe"), expected: true},
		{name: "connection reset", err: errors.New("read tcp: connection reset by peer"), expected: true},
		{name: "context canceled", err: context.Canceled, expected: true},
		{name: "wrapped context canceled", err: fmt.Errorf("stream aborted: %w", context.Canceled), expected: true},
		{name: "unexpected eof", err: errors.New("unexpected EOF"), expected: false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := IsClientDisconnectError(tt.err); got != tt.expected {
				t.Errorf("IsClientDisconnectError(%v) = %v, want %v", tt.err, got, tt.expected)
			}
		})
	}
}

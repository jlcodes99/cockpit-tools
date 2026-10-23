package helps

import (
	"context"
	"errors"
	"testing"
	"time"

	api "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestCanceledCodexStreamKeepsErrorWithoutReceiver(t *testing.T) {
	for _, capacity := range []int{1, 3} {
		ctx, cancel := context.WithCancel(t.Context())
		cancel()
		out := make(chan api.StreamChunk, capacity)
		for index := 0; index < capacity; index++ {
			out <- api.StreamChunk{Payload: []byte("canceled normal output")}
		}
		failure := errors.New("confirmed quota failure")
		done := make(chan bool, 1)
		go func() { done <- SendCodexWebsocketChunk(ctx, out, api.StreamChunk{Err: failure}) }()
		select {
		case sent := <-done:
			if !sent {
				t.Fatal("confirmed failure dropped")
			}
		case <-time.After(time.Second):
			t.Fatal("failure waited for a downstream receiver")
		}
		if len(out) != 1 || (<-out).Err != failure {
			t.Fatal("normal output occupied the reserved failure slot")
		}
		if SendCodexWebsocketChunk(ctx, out, api.StreamChunk{Payload: []byte("late")}) || len(out) != 0 {
			t.Fatal("canceled ordinary output was forwarded")
		}
		if SendCodexWebsocketChunk(ctx, out, api.StreamChunk{Err: context.Canceled}) || len(out) != 0 {
			t.Fatal("plain cancellation was emitted as an upstream failure")
		}
	}
}

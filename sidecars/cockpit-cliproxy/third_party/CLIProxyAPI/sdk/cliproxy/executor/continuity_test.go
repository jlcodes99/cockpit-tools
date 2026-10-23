package executor

import (
	"context"
	"errors"
	"net/http"
	"sync"
	"testing"
	"time"

	"github.com/tidwall/gjson"
)

type continuityTestHook struct {
	mu       sync.Mutex
	binds    []ContinuityBindRequest
	observed []ContinuityObserveRequest
	finished []ContinuityFinishRequest
}

func (h *continuityTestHook) Prepare(context.Context, ContinuityPrepareRequest) (ContinuityPrepareResponse, error) {
	return ContinuityPrepareResponse{Ticket: "ticket", PreferredAccountID: "original"}, nil
}
func (h *continuityTestHook) Bind(_ context.Context, r ContinuityBindRequest) (ContinuityBindResponse, error) {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.binds = append(h.binds, r)
	return ContinuityBindResponse{Ticket: r.Ticket, AttemptID: r.AttemptID, Body: r.Body, Headers: r.Headers}, nil
}
func (h *continuityTestHook) Observe(_ context.Context, r ContinuityObserveRequest) (ContinuityObserveResponse, error) {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.observed = append(h.observed, r)
	return ContinuityObserveResponse{NextSequence: r.Sequence + 1, Terminal: len(r.Events) > 0}, nil
}
func (h *continuityTestHook) Finish(ctx context.Context, r ContinuityFinishRequest) error {
	if ctx.Err() != nil {
		return ctx.Err()
	}
	h.mu.Lock()
	defer h.mu.Unlock()
	h.finished = append(h.finished, r)
	return nil
}

func TestContinuityAutomaticAttemptsPreserveCanonicalHistoryAndIndependentIDs(t *testing.T) {
	h := &continuityTestHook{}
	canonical := []byte(`{"model":"original-model","previous_response_id":"original-response","client_metadata":{"thread_id":"original-thread"},"input":[{"type":"reasoning","id":"original-id","encrypted_content":"synthetic-cipher"}]}`)
	ctx, _, err := PrepareContinuity(WithContinuityHook(context.Background(), h), ContinuityPrepareRequest{Body: canonical})
	if err != nil {
		t.Fatal(err)
	}
	var wg sync.WaitGroup
	var mu sync.Mutex
	var attempts []*ContinuityAttempt
	for range 8 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			attempt, _, _, err := BindContinuity(ctx, "original", http.Header{}, []byte(`{"model":"normalized-model","input":[]}`))
			if err == nil {
				mu.Lock()
				attempts = append(attempts, attempt)
				mu.Unlock()
			}
		}()
	}
	wg.Wait()
	if len(attempts) != 8 || len(h.binds) != 8 {
		t.Fatalf("automatic sends=%d binds=%d, want eight", len(attempts), len(h.binds))
	}
	seen := map[string]bool{}
	for _, binding := range h.binds {
		if seen[binding.AttemptID] {
			t.Fatal("automatic attempts reused a binding ID")
		}
		seen[binding.AttemptID] = true
		if gjson.GetBytes(binding.Body, "input").Raw != gjson.GetBytes(canonical, "input").Raw {
			t.Fatalf("retry lost opaque history: %s", binding.Body)
		}
	}
	bound := h.binds[0].Body
	if gjson.GetBytes(bound, "input").Raw != gjson.GetBytes(canonical, "input").Raw || gjson.GetBytes(bound, "previous_response_id").String() != "original-response" || gjson.GetBytes(bound, "model").String() != "normalized-model" {
		t.Fatalf("canonical history was changed: %s", bound)
	}
	if err := attempts[0].Finish(ctx, errors.New("upstream failed")); err != nil {
		t.Fatal(err)
	}
	if h.finished[0].Status != "failed" {
		t.Fatalf("finish=%+v", h.finished[0])
	}
}

func TestContinuitySequencesAndCancellationDoNotClaimSuccess(t *testing.T) {
	h := &continuityTestHook{}
	ctx, _, err := PrepareContinuity(WithContinuityHook(context.Background(), h), ContinuityPrepareRequest{Body: []byte(`{"input":[]}`)})
	if err != nil {
		t.Fatal(err)
	}
	attempt, _, _, err := BindContinuity(ctx, "original", nil, []byte(`{"input":[]}`))
	if err != nil {
		t.Fatal(err)
	}
	if err := attempt.Observe(ctx, http.Header{"X-Codex-Turn-State": []string{"synthetic-state"}}, nil); err != nil {
		t.Fatal(err)
	}
	if err := attempt.Observe(ctx, nil, []byte(`{"type":"response.created","response":{"id":"resp"}}`)); err != nil {
		t.Fatal(err)
	}
	cancelled, cancel := context.WithCancel(ctx)
	cancel()
	if err := attempt.Finish(cancelled, nil); err != nil {
		t.Fatal(err)
	}
	if err := attempt.Finish(ctx, nil); err != nil {
		t.Fatal(err)
	}
	if len(h.finished) != 1 || h.finished[0].Status != "cancelled" || h.finished[0].Sequence != 3 {
		t.Fatalf("finish=%+v", h.finished)
	}
	if h.observed[0].Sequence != 1 || h.observed[1].Sequence != 2 {
		t.Fatalf("observations=%+v", h.observed)
	}
	if err := attempt.Observe(ctx, nil, []byte(`{"type":"response.completed"}`)); err == nil {
		t.Fatal("finished attempt accepted late event")
	}
}

func TestContinuityUnboundReservationReleaseIsIdempotent(t *testing.T) {
	h := &continuityTestHook{}
	ctx, _, err := PrepareContinuity(WithContinuityHook(context.Background(), h), ContinuityPrepareRequest{Body: []byte(`{"input":[]}`)})
	if err != nil {
		t.Fatal(err)
	}
	for range 2 {
		if err := FinishUnboundContinuity(ctx); err != nil {
			t.Fatal(err)
		}
	}
	if len(h.finished) != 1 || h.finished[0].AttemptID != "" || h.finished[0].Sequence != 1 || h.finished[0].Status != "failed" {
		t.Fatalf("reservation finish=%+v", h.finished)
	}
	if _, _, _, err := BindContinuity(ctx, "original", nil, []byte(`{"input":[]}`)); err == nil {
		t.Fatal("finished reservation accepted a send")
	}
}

type continuityReleaseHook struct {
	continuityTestHook
	released chan ContinuityReleaseRequest
}

func (h *continuityReleaseHook) Release(_ context.Context, r ContinuityReleaseRequest) error {
	h.released <- r
	return nil
}

func TestContinuityStreamReleaseFollowsSourceLifetime(t *testing.T) {
	for _, cancelled := range []bool{false, true} {
		t.Run(map[bool]string{false: "source-close", true: "cancel"}[cancelled], func(t *testing.T) {
			hook := &continuityReleaseHook{released: make(chan ContinuityReleaseRequest, 2)}
			ctx, _, err := PrepareContinuity(WithContinuityHook(context.Background(), hook), ContinuityPrepareRequest{Body: []byte(`{"input":[]}`)})
			if err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithCancel(ctx)
			defer cancel()
			source := make(chan StreamChunk, 1)
			result := ReleaseContinuityOnStreamEnd(ctx, &StreamResult{Chunks: source})
			select {
			case <-hook.released:
				t.Fatal("live stream released early")
			default:
			}
			source <- StreamChunk{Payload: []byte("one")}
			if chunk := <-result.Chunks; string(chunk.Payload) != "one" {
				t.Fatalf("chunk=%+v", chunk)
			}
			select {
			case <-hook.released:
				t.Fatal("stream released after only first chunk")
			default:
			}
			if cancelled {
				cancel()
			} else {
				close(source)
			}
			select {
			case _, ok := <-result.Chunks:
				if ok {
					t.Fatal("unexpected tail")
				}
			case <-time.After(time.Second):
				t.Fatal("stream did not close")
			}
			select {
			case r := <-hook.released:
				if r.Ticket != "ticket" {
					t.Fatalf("release=%+v", r)
				}
			default:
				t.Fatal("missing request release")
			}
			if err := ReleaseContinuity(ctx); err != nil {
				t.Fatal(err)
			}
			select {
			case <-hook.released:
				t.Fatal("duplicate release")
			default:
			}
		})
	}
}

func TestContinuityRequiredDefaultsDoNotEraseOpaqueOrphanHistory(t *testing.T) {
	body := []byte(`{"input":[{"type":"custom_tool_call_output","id":"opaque_original","output":{"encrypted_content":"synthetic-orphan"}}]}`)
	normalized := NormalizeResponsesCallIDs(body, true)
	if gjson.GetBytes(normalized, "input.0.id").String() != "opaque_original" || gjson.GetBytes(normalized, "input.0.output.encrypted_content").String() != "synthetic-orphan" {
		t.Fatalf("continuity erased unverified opaque history: %s", normalized)
	}
	if len(gjson.GetBytes(NormalizeResponsesCallIDs(body), "input").Array()) != 0 {
		t.Fatal("ordinary executor orphan compatibility changed")
	}
}

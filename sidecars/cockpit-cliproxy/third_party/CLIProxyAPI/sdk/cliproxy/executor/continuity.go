package executor

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"sync"

	"github.com/google/uuid"
	log "github.com/sirupsen/logrus"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// ContinuityHook owns durable Responses identity and continuation transactions.
// Unlike diagnostics, these callbacks are acknowledged before forwarding output.
// Implementations must authenticate and bound any local control-plane I/O.
type ContinuityHook interface {
	Prepare(context.Context, ContinuityPrepareRequest) (ContinuityPrepareResponse, error)
	Bind(context.Context, ContinuityBindRequest) (ContinuityBindResponse, error)
	Observe(context.Context, ContinuityObserveRequest) (ContinuityObserveResponse, error)
	Finish(context.Context, ContinuityFinishRequest) error
}

// ContinuityFinalizer releases request plans after all credential attempts end.
// It is optional for existing hook implementations.
type ContinuityFinalizer interface {
	Release(context.Context, ContinuityReleaseRequest) error
}

type ContinuityReleaseRequest struct {
	Ticket string `json:"ticket"`
}

type ContinuityPrepareRequest struct {
	RequestID    string          `json:"request_id"`
	Caller       string          `json:"caller"`
	Headers      http.Header     `json:"headers"`
	Body         json.RawMessage `json:"body"`
	Compact      bool            `json:"compact"`
	AllowRebuild bool            `json:"allow_rebuild"`
	SourceFormat string          `json:"-"`
	Model        string          `json:"-"`
}

type ContinuityPrepareResponse struct {
	Ticket string `json:"ticket"`
	// PreferredAccountID is the SDK Auth.ID; the hook translates host account IDs.
	PreferredAccountID string          `json:"preferred_account_id"`
	SessionID          string          `json:"session_id"`
	ThreadID           string          `json:"thread_id"`
	ParentThreadID     string          `json:"parent_thread_id"`
	SelfContained      bool            `json:"self_contained"`
	RequireCodex       bool            `json:"-"`
	Body               json.RawMessage `json:"body"`
	Headers            http.Header     `json:"headers"`
}

type ContinuityBindRequest struct {
	Ticket    string `json:"ticket"`
	AttemptID string `json:"attempt_id"`
	// AccountID is the SDK Auth.ID, never a client-provided account header.
	AccountID string `json:"account_id"`
	OwnerID   string `json:"owner_id,omitempty"`
	// Enabled only by a selected OAuth HTTP transport which cannot resolve previous IDs.
	ExpandHTTPPrevious bool            `json:"expand_http_previous,omitempty"`
	Headers            http.Header     `json:"headers"`
	Body               json.RawMessage `json:"body"`
}

type ContinuityBindResponse struct {
	Ticket         string          `json:"ticket"`
	AttemptID      string          `json:"attempt_id"`
	Headers        http.Header     `json:"headers"`
	Body           json.RawMessage `json:"body"`
	RebuildApplied bool            `json:"rebuild_applied"`
}

type ContinuityObserveRequest struct {
	Ticket    string            `json:"ticket"`
	AttemptID string            `json:"attempt_id"`
	Sequence  uint64            `json:"sequence"`
	Headers   http.Header       `json:"headers,omitempty"`
	Events    []json.RawMessage `json:"events"`
}

type ContinuityObserveResponse struct {
	NextSequence uint64 `json:"next_sequence"`
	Terminal     bool   `json:"terminal"`
}

type ContinuityFinishRequest struct {
	Ticket    string `json:"ticket"`
	AttemptID string `json:"attempt_id"`
	Sequence  uint64 `json:"sequence"`
	Status    string `json:"status"`
	ErrorCode string `json:"error_code,omitempty"`
}

type continuityHookKey struct{}
type continuityPreparationErrorKey struct{}
type continuityHookValue struct{ hook ContinuityHook }
type continuityPlanKey struct{}
type continuityPlan struct {
	hook            ContinuityHook
	prepared        ContinuityPrepareResponse
	canonical       json.RawMessage
	compact         bool
	mu              sync.Mutex
	bound           bool
	unboundFinished bool
	released        bool
}

func WithContinuityHook(ctx context.Context, hook ContinuityHook) context.Context {
	if ctx == nil {
		ctx = context.Background()
	}
	return context.WithValue(ctx, continuityHookKey{}, continuityHookValue{hook: hook})
}

func ContinuityHookFromContext(ctx context.Context) ContinuityHook {
	if ctx == nil {
		return nil
	}
	value, _ := ctx.Value(continuityHookKey{}).(continuityHookValue)
	return value.hook
}

// WithContinuityPreparationError lets a mixed-protocol route retain its native
// providers while fencing Codex execution before legacy history sanitization.
func WithContinuityPreparationError(ctx context.Context, err error) context.Context {
	return context.WithValue(ctx, continuityPreparationErrorKey{}, err)
}

func ContinuityPreparationError(ctx context.Context) error {
	if ctx == nil {
		return nil
	}
	err, _ := ctx.Value(continuityPreparationErrorKey{}).(error)
	return err
}

func continuityPlanFrom(ctx context.Context) *continuityPlan {
	if ctx == nil {
		return nil
	}
	plan, _ := ctx.Value(continuityPlanKey{}).(*continuityPlan)
	return plan
}

func ContinuityActive(ctx context.Context) bool { return continuityPlanFrom(ctx) != nil }

// FinishUnboundContinuity releases a reservation when selection, credential
// preparation or request validation failed before any upstream binding existed.
func FinishUnboundContinuity(ctx context.Context) error {
	plan := continuityPlanFrom(ctx)
	if plan == nil {
		return nil
	}
	plan.mu.Lock()
	if plan.bound || plan.unboundFinished {
		plan.mu.Unlock()
		return nil
	}
	plan.unboundFinished = true
	plan.mu.Unlock()
	status := "failed"
	if ctx.Err() != nil {
		status = "cancelled"
	}
	return plan.hook.Finish(context.WithoutCancel(ctx), ContinuityFinishRequest{Ticket: plan.prepared.Ticket, Sequence: 1, Status: status, ErrorCode: "continuity_not_sent"})
}

// ReleaseContinuity never deletes durable conversation history. It only frees
// this logical request's control-plane reservation and transient binding plan.
func ReleaseContinuity(ctx context.Context) error {
	plan := continuityPlanFrom(ctx)
	if plan == nil {
		return nil
	}
	plan.mu.Lock()
	if plan.released {
		plan.mu.Unlock()
		return nil
	}
	plan.released = true
	plan.mu.Unlock()
	finalizer, ok := plan.hook.(ContinuityFinalizer)
	if !ok {
		return nil
	}
	return finalizer.Release(context.WithoutCancel(ctx), ContinuityReleaseRequest{Ticket: plan.prepared.Ticket})
}

// ReleaseContinuityOnStreamEnd has exactly one consumer for the original stream.
// Ownership follows the returned stream until its source closes or the caller
// cancels; returning ExecuteStream itself is not an execution terminal.
func ReleaseContinuityOnStreamEnd(ctx context.Context, result *StreamResult) *StreamResult {
	if !ContinuityActive(ctx) || result == nil || result.Chunks == nil {
		return result
	}
	out := make(chan StreamChunk)
	go func() {
		defer close(out)
		defer func() {
			if err := ReleaseContinuity(ctx); err != nil {
				log.WithError(err).Warn("continuity plan release failed")
			}
		}()
		for {
			select {
			case <-ctx.Done():
				return
			case chunk, ok := <-result.Chunks:
				if !ok {
					return
				}
				select {
				case out <- chunk:
				case <-ctx.Done():
					return
				}
			}
		}
	}()
	copy := *result
	copy.Chunks = out
	return &copy
}

// PrepareContinuity does not change the caller's protocol or original payload.
func PrepareContinuity(ctx context.Context, request ContinuityPrepareRequest) (context.Context, ContinuityPrepareResponse, error) {
	if plan := continuityPlanFrom(ctx); plan != nil {
		return ctx, plan.prepared, nil
	}
	hook := ContinuityHookFromContext(ctx)
	if hook == nil {
		return ctx, ContinuityPrepareResponse{}, nil
	}
	if request.RequestID == "" {
		request.RequestID = uuid.NewString()
	}
	prepared, err := hook.Prepare(ctx, request)
	if err != nil {
		return ctx, prepared, err
	}
	if prepared.Ticket == "" {
		return ctx, prepared, nil
	}
	return context.WithValue(ctx, continuityPlanKey{}, &continuityPlan{hook: hook, prepared: prepared, canonical: append(json.RawMessage(nil), request.Body...), compact: request.Compact}), prepared, nil
}

// ContinuityAttempt is one actual upstream send. A successor attempt has an
// independent sequence and binding; late callbacks cannot address its binding.
type ContinuityAttempt struct {
	mu             sync.Mutex
	hook           ContinuityHook
	ticket, id     string
	next           uint64
	terminal       bool
	successful     bool
	compact        bool
	finished       bool
	eventTransform func([]byte) []byte
	output         continuityOutput
}

// SetEventTransform allows a selected transport to add authoritative response
// metadata before persistence. It does not change canonical input or wire logs.
func (a *ContinuityAttempt) SetEventTransform(transform func([]byte) []byte) {
	if a == nil {
		return
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	if !a.finished {
		a.eventTransform = transform
	}
}

func BindContinuity(ctx context.Context, accountID string, headers http.Header, body []byte, expandHTTPPrevious ...bool) (*ContinuityAttempt, http.Header, []byte, error) {
	plan := continuityPlanFrom(ctx)
	if plan == nil {
		return nil, headers, body, nil
	}
	plan.mu.Lock()
	if plan.released || plan.unboundFinished {
		plan.mu.Unlock()
		return nil, nil, nil, errors.New("continuity request already finished")
	}
	plan.bound = true
	plan.mu.Unlock()
	id := uuid.NewString()
	// Executable options may be normalized after Prepare, but identity-bearing
	// history must remain byte-preserved until the authoritative binding rewrite.
	for _, field := range []string{"input", "previous_response_id", "client_metadata", "conversation", "conversation_id", "prompt_cache_key", "session_id", "thread_id", "parent_thread_id"} {
		value := gjson.GetBytes(plan.canonical, field)
		var err error
		if value.Exists() {
			body, err = sjson.SetRawBytes(body, field, []byte(value.Raw))
		} else {
			body, err = sjson.DeleteBytes(body, field)
		}
		if err != nil {
			return nil, nil, nil, err
		}
	}
	expandPrevious := len(expandHTTPPrevious) > 0 && expandHTTPPrevious[0]
	bound, err := plan.hook.Bind(ctx, ContinuityBindRequest{Ticket: plan.prepared.Ticket, AttemptID: id, AccountID: accountID, Headers: headers.Clone(), Body: append(json.RawMessage(nil), body...), ExpandHTTPPrevious: expandPrevious})
	if err != nil {
		return nil, nil, nil, err
	}
	if bound.Ticket != "" && bound.Ticket != plan.prepared.Ticket || bound.AttemptID != "" && bound.AttemptID != id {
		return nil, nil, nil, errors.New("continuity returned a different attempt identity")
	}
	if !json.Valid(bound.Body) {
		return nil, nil, nil, errors.New("continuity returned invalid Responses payload")
	}
	return &ContinuityAttempt{hook: plan.hook, ticket: plan.prepared.Ticket, id: id, next: 1, compact: plan.compact}, bound.Headers.Clone(), append([]byte(nil), bound.Body...), nil
}

func (a *ContinuityAttempt) Observe(ctx context.Context, headers http.Header, event []byte) error {
	if a == nil {
		return nil
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.finished {
		return errors.New("continuity attempt already finished")
	}
	request := ContinuityObserveRequest{Ticket: a.ticket, AttemptID: a.id, Sequence: a.next, Headers: headers.Clone(), Events: []json.RawMessage{}}
	if len(event) > 0 {
		if a.eventTransform != nil {
			event = a.eventTransform(event)
		}
		if !json.Valid(event) {
			a.output.discard()
			return errors.New("invalid upstream Responses event")
		}
		if ctx != nil && ctx.Err() != nil || a.terminal {
			a.output.discard()
		} else {
			event = a.output.observe(event)
		}
		request.Events = append(request.Events, append(json.RawMessage(nil), event...))
	}
	ack, err := a.hook.Observe(ctx, request)
	if err != nil {
		a.output.discard()
		return err
	}
	if ack.NextSequence != a.next+1 {
		a.output.discard()
		return fmt.Errorf("continuity sequence acknowledgement mismatch: %d", ack.NextSequence)
	}
	a.next = ack.NextSequence
	a.terminal = a.terminal || ack.Terminal
	if ack.Terminal {
		a.output.discard()
	}
	if ack.Terminal && len(event) > 0 {
		response := gjson.ParseBytes(event)
		if nested := response.Get("response"); nested.IsObject() {
			response = nested
		}
		status := response.Get("status").String()
		noError := !gjson.GetBytes(event, "error").Exists() || gjson.GetBytes(event, "error").Type == gjson.Null
		noError = noError && (!response.Get("error").Exists() || response.Get("error").Type == gjson.Null)
		a.successful = noError && response.Get("id").String() != "" && (status == "completed" || a.compact && response.Get("output").IsArray() && status != "failed" && status != "incomplete" && status != "cancelled")
	}
	return nil
}

// Finish cannot infer success from EOF. Only an acknowledged raw terminal can
// authorize success; the durable hook validates and commits that terminal itself.
func (a *ContinuityAttempt) Finish(ctx context.Context, cause error) error {
	return a.finish(ctx, cause, false)
}

// FinishNotSent is only for trusted local preparation failures before a request
// is sent. Transport errors with an uncertain send status must use Finish.
func (a *ContinuityAttempt) FinishNotSent(ctx context.Context, cause error) error {
	return a.finish(ctx, cause, true)
}

func (a *ContinuityAttempt) finish(ctx context.Context, cause error, notSent bool) error {
	if a == nil {
		return nil
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.finished {
		return nil
	}
	a.finished = true
	a.output.discard()
	status, code := "failed", "continuity_incomplete"
	if cause != nil {
		code = "continuity_upstream_failed"
		if notSent && a.next == 1 && !a.terminal {
			code = "continuity_upstream_not_sent"
		}
	}
	if ctx != nil && ctx.Err() != nil {
		status, code = "cancelled", "continuity_cancelled"
	} else if cause == nil && a.terminal && a.successful {
		status, code = "succeeded", ""
	}
	if ctx == nil {
		ctx = context.Background()
	}
	return a.hook.Finish(context.WithoutCancel(ctx), ContinuityFinishRequest{Ticket: a.ticket, AttemptID: a.id, Sequence: a.next, Status: status, ErrorCode: code})
}

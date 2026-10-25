package helps

import (
	"bufio"
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

// BindCodexContinuity preserves the separately managed installation profile.
// Applying that profile changes only installation fields; session, thread IDs
// and encrypted history remain exclusively owned by the continuity binding.
func BindCodexContinuity(ctx context.Context, auth *cliproxyauth.Auth, headers http.Header, body []byte) (*cliproxyexecutor.ContinuityAttempt, http.Header, []byte, error) {
	return bindCodexContinuity(ctx, auth, headers, body, false)
}

func bindCodexContinuity(ctx context.Context, auth *cliproxyauth.Auth, headers http.Header, body []byte, expandHTTPPrevious bool) (*cliproxyexecutor.ContinuityAttempt, http.Header, []byte, error) {
	accountID := ""
	if auth != nil {
		accountID = auth.ID
	}
	attempt, headers, next, err := cliproxyexecutor.BindContinuity(ctx, accountID, headers, body, expandHTTPPrevious)
	if err != nil || attempt == nil {
		return attempt, headers, next, err
	}
	next, err = ApplyManagedIdentity(auth, headers, next)
	if err != nil {
		return nil, nil, nil, errors.Join(err, attempt.FinishNotSent(ctx, err))
	}
	return attempt, headers, next, nil
}

// BindContinuityHTTPRequest runs after protocol normalization and before a send.
// Its returned headers are authoritative for identity, while transport credentials
// remain those of the already selected auth.
func BindContinuityHTTPRequest(ctx context.Context, auth *cliproxyauth.Auth, req *http.Request, body []byte) (*cliproxyexecutor.ContinuityAttempt, []byte, error) {
	if !cliproxyexecutor.ContinuityActive(ctx) {
		return nil, body, nil
	}
	expandPrevious := IsCodexOAuthAuth(auth) && req.URL != nil && strings.HasSuffix(req.URL.Path, "/responses")
	attempt, headers, next, err := bindCodexContinuity(ctx, auth, req.Header, body, expandPrevious)
	if err != nil {
		return nil, nil, err
	}
	req.Header = headers
	req.Body = io.NopCloser(bytes.NewReader(next))
	req.ContentLength = int64(len(next))
	req.GetBody = func() (io.ReadCloser, error) { return io.NopCloser(bytes.NewReader(next)), nil }
	return attempt, next, nil
}

// ObserveContinuityHTTPResponse observes raw wire events before any executor
// repair, identity exposure or client-protocol translation. Reading an event
// releases its bytes only after the durable observer acknowledges it.
func ObserveContinuityHTTPResponse(ctx context.Context, attempt *cliproxyexecutor.ContinuityAttempt, resp *http.Response, sendErr error) error {
	if attempt == nil {
		return sendErr
	}
	if sendErr != nil {
		return errors.Join(sendErr, attempt.Finish(ctx, sendErr))
	}
	if resp == nil || resp.Body == nil {
		err := errors.New("continuity upstream response has no body")
		return errors.Join(err, attempt.Finish(ctx, err))
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return attempt.Finish(ctx, fmt.Errorf("upstream status %d", resp.StatusCode))
	}
	if err := attempt.Observe(ctx, resp.Header, nil); err != nil {
		_ = resp.Body.Close()
		return errors.Join(err, attempt.Finish(ctx, err))
	}
	resp.Body = &continuityResponseBody{ctx: ctx, attempt: attempt, source: resp.Body, reader: bufio.NewReader(resp.Body)}
	return nil
}

const continuityMaximumEventBytes = 52_428_800

type continuityResponseBody struct {
	ctx      context.Context
	attempt  *cliproxyexecutor.ContinuityAttempt
	source   io.ReadCloser
	reader   *bufio.Reader
	json     bool
	detected bool
	pending  []byte
	err      error
	done     bool
}

func (r *continuityResponseBody) Read(dst []byte) (int, error) {
	if len(dst) == 0 {
		return 0, nil
	}
	if len(r.pending) == 0 && !r.done {
		if !r.detected {
			// Some compatible upstreams label SSE as application/json or omit
			// Content-Type. Preserve the executor's existing wire compatibility.
			var prefix []byte
			for {
				b, err := r.reader.ReadByte()
				if err != nil {
					r.err = err
					r.done = true
					break
				}
				prefix = append(prefix, b)
				if b != ' ' && b != '\t' && b != '\r' && b != '\n' {
					r.json = b == '{' || b == '['
					r.detected = true
					r.reader = bufio.NewReader(io.MultiReader(bytes.NewReader(prefix), r.reader))
					break
				}
				if len(prefix) > continuityMaximumEventBytes {
					r.err = errors.New("continuity upstream prefix exceeds size limit")
					r.done = true
					break
				}
			}
		}
		if r.done {
			return 0, r.err
		}
		var frame []byte
		if r.json {
			frame, r.err = io.ReadAll(io.LimitReader(r.reader, continuityMaximumEventBytes+1))
			r.done = true
		} else {
			for {
				line, err := r.reader.ReadBytes('\n')
				frame = append(frame, line...)
				if err != nil {
					r.err = err
					r.done = true
					break
				}
				if len(frame) > continuityMaximumEventBytes || len(bytes.TrimSpace(line)) == 0 {
					break
				}
			}
		}
		if len(frame) > continuityMaximumEventBytes {
			r.err = errors.New("continuity upstream event exceeds size limit")
			r.done = true
			frame = nil
		}
		if len(frame) > 0 {
			var event []byte
			if r.json {
				event = bytes.TrimSpace(frame)
			} else {
				for _, line := range bytes.Split(frame, []byte("\n")) {
					if bytes.HasPrefix(line, []byte("data:")) {
						if len(event) > 0 {
							event = append(event, '\n')
						}
						event = append(event, bytes.TrimSpace(line[5:])...)
					}
				}
			}
			if len(event) > 0 && !bytes.Equal(event, []byte("[DONE]")) {
				var observeErr error
				if r.ctx.Err() != nil {
					// Preserve the executor's cancellation drain for authoritative
					// quota failures, without publishing or committing late success.
					if !CodexWebsocketFailureCandidate(event) {
						observeErr = r.ctx.Err()
					}
				} else {
					observeErr = r.attempt.Observe(r.ctx, nil, event)
				}
				if observeErr != nil {
					r.err = observeErr
					r.done = true
					frame = nil
				}
			}
			r.pending = frame
		}
	}
	if len(r.pending) > 0 {
		n := copy(dst, r.pending)
		r.pending = r.pending[n:]
		return n, nil
	}
	if r.err != nil {
		return 0, r.err
	}
	if r.done {
		return 0, io.EOF
	}
	return r.Read(dst)
}

func (r *continuityResponseBody) Close() error {
	err := r.source.Close()
	cause := r.err
	if errors.Is(cause, io.EOF) {
		cause = nil
	}
	return errors.Join(err, r.attempt.Finish(r.ctx, cause))
}

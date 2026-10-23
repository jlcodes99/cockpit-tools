package executor

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
	"github.com/tidwall/gjson"
)

func TestCodexStreamCauseClassificationAndClientEvent(t *testing.T) {
	reader, writer := net.Pipe()
	defer reader.Close()
	defer writer.Close()
	if err := reader.SetReadDeadline(time.Now().Add(-time.Second)); err != nil {
		t.Fatal(err)
	}
	_, timeout := reader.Read(make([]byte, 1))
	for _, tc := range []struct {
		name, code, message string
		cause               error
		status              int
	}{
		{"clean-close", "incomplete_upstream_stream", "stream closed before response.completed", nil, 502},
		{"eof", "incomplete_upstream_stream", "stream closed before response.completed", io.EOF, 502},
		{"read-error", "upstream_stream_error", "unexpected EOF", fmt.Errorf("read https://user:pass@local.test/?access_token=secret-token: %w; Authorization=Bearer secret-bearer", io.ErrUnexpectedEOF), 502},
		{"real-timeout", "request_timeout", "i/o timeout", timeout, 408},
		{"deadline", "request_timeout", "context deadline exceeded", fmt.Errorf("body read: %w", context.DeadlineExceeded), 408},
		{"timeout-text-is-not-timeout", "upstream_stream_error", "timeout", errors.New("peer reported timeout but reset the connection"), 502},
	} {
		t.Run(tc.name, func(t *testing.T) {
			err := newCodexIncompleteStreamError(tc.cause)
			if err.StatusCode() != tc.status || !err.IsRequestScoped() || tc.cause != nil && !errors.Is(err, tc.cause) {
				t.Fatalf("incorrect status, request scope or cause: %#v", err)
			}
			event, payload := handlers.BuildOpenAIResponsesStreamTerminalEvent(err.StatusCode(), err, 3)
			if event != "response.failed" || gjson.GetBytes(payload, "response.error.code").String() != tc.code {
				t.Fatalf("incorrect client event: %s %s", event, payload)
			}
			message := gjson.GetBytes(payload, "response.error.message").String()
			if !strings.Contains(message, tc.message) || strings.Contains(message, "stream error:") || strings.Contains(message, "stream disconnected before completion:") {
				t.Fatalf("cause lost or wrapped twice: %s", payload)
			}
			for _, secret := range []string{"user:pass", "secret-token", "secret-bearer"} {
				if strings.Contains(string(payload), secret) {
					t.Fatalf("transport credentials leaked: %s", payload)
				}
			}
		})
	}
}

func TestCodexHTTPStreamLocalTruncationRetainsUnexpectedEOF(t *testing.T) {
	for _, buffering := range []bool{false, true} {
		for _, stream := range []bool{false, true} {
			t.Run(fmt.Sprintf("buffering=%v/stream=%v", buffering, stream), func(t *testing.T) {
				server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
					w.Header().Set("Content-Type", "text/event-stream")
					w.Header().Set("Content-Length", "99999")
					_, _ = fmt.Fprintf(w, "data: %s\n\n", codexCreatedEvent)
				}))
				defer server.Close()
				req, opts := codexTestRequest()
				executor := NewCodexExecutor(codexBufferingConfig(buffering))
				var err error
				if stream {
					result, errStream := executor.ExecuteStream(t.Context(), codexTestAuth(server.URL), req, opts)
					err = errStream
					if result != nil {
						_, err = drainChunks(result)
					}
				} else {
					_, err = executor.Execute(t.Context(), codexTestAuth(server.URL), req, opts)
				}
				if err == nil || !errors.Is(err, io.ErrUnexpectedEOF) || statusCodeFromTestError(t, err) != 502 {
					t.Fatalf("local truncated response lost cause: %v", err)
				}
				assertRequestScopedTestError(t, err)
				if gjson.Get(err.Error(), "error.code").String() != "upstream_stream_error" || !strings.Contains(err.Error(), "unexpected EOF") {
					t.Fatalf("incorrect transport error: %v", err)
				}
			})
		}
	}
}

func TestCodexWebsocketPrematureCloseRetainsCause(t *testing.T) {
	for _, buffering := range []bool{false, true} {
		for _, stream := range []bool{false, true} {
			t.Run(fmt.Sprintf("buffering=%v/stream=%v", buffering, stream), func(t *testing.T) {
				server := codexWebsocketServer(t, codexCreatedEvent)
				defer server.Close()
				req, opts := codexWebsocketRequest()
				executor := NewCodexWebsocketsExecutor(codexBufferingConfig(buffering))
				var err error
				if stream {
					result, errStream := executor.ExecuteStream(t.Context(), codexTestAuth(server.URL), req, opts)
					err = errStream
					if result != nil {
						_, err = drainChunks(result)
					}
				} else {
					_, err = executor.Execute(t.Context(), codexTestAuth(server.URL), req, opts)
				}
				if err == nil || statusCodeFromTestError(t, err) != 502 || !strings.Contains(err.Error(), "unexpected EOF") {
					t.Fatalf("websocket close lost cause: %v", err)
				}
				assertRequestScopedTestError(t, err)
			})
		}
	}
}

func TestCodexHTTPStreamRealReadTimeout(t *testing.T) {
	for _, buffering := range []bool{false, true} {
		for _, stream := range []bool{false, true} {
			t.Run(fmt.Sprintf("buffering=%v/stream=%v", buffering, stream), func(t *testing.T) {
				reader, writer := net.Pipe()
				defer reader.Close()
				defer writer.Close()
				if err := reader.SetReadDeadline(time.Now().Add(-time.Second)); err != nil {
					t.Fatal(err)
				}
				ctx := context.WithValue(t.Context(), "cliproxy.roundtripper", roundTripperFunc(func(req *http.Request) (*http.Response, error) {
					return &http.Response{StatusCode: 200, Header: http.Header{"Content-Type": {"text/event-stream"}}, Request: req,
						Body: io.NopCloser(io.MultiReader(strings.NewReader("data: "+codexCreatedEvent+"\n\n"), reader))}, nil
				}))
				req, opts := codexTestRequest()
				executor := NewCodexExecutor(codexBufferingConfig(buffering))
				var err error
				if stream {
					result, errStream := executor.ExecuteStream(ctx, codexTestAuth("http://codex.test"), req, opts)
					err = errStream
					if result != nil {
						_, err = drainChunks(result)
					}
				} else {
					_, err = executor.Execute(ctx, codexTestAuth("http://codex.test"), req, opts)
				}
				var timeout net.Error
				if err == nil || statusCodeFromTestError(t, err) != 408 || !errors.As(err, &timeout) || !timeout.Timeout() || gjson.Get(err.Error(), "error.code").String() != "request_timeout" {
					t.Fatalf("proven read timeout lost its classification or cause: %v", err)
				}
				assertRequestScopedTestError(t, err)
			})
		}
	}
}

type codexStreamCauseUsagePlugin struct {
	records chan usage.Record
}

func (p *codexStreamCauseUsagePlugin) HandleUsage(_ context.Context, record usage.Record) {
	if strings.HasPrefix(record.AuthID, "stream-cause-usage/") {
		select {
		case p.records <- record:
		default:
		}
	}
}

func TestCodexWebsocketSessionDisconnectKeepsActiveCauseButNotCompletedFailure(t *testing.T) {
	plugin := &codexStreamCauseUsagePlugin{records: make(chan usage.Record, 8)}
	usage.RegisterNamedPlugin("codex-stream-cause-usage-test", plugin)
	for _, buffering := range []bool{false, true} {
		for _, completed := range []bool{false, true} {
			t.Run(fmt.Sprintf("buffering=%v/completed=%v", buffering, completed), func(t *testing.T) {
				frames := []string{codexCreatedEvent}
				if completed {
					frames = append(frames, codexCompletedEventBody)
				}
				server := codexWebsocketServer(t, frames...)
				defer server.Close()
				executor := NewCodexWebsocketsExecutor(codexBufferingConfig(buffering))
				executor.store = &codexWebsocketSessionStore{sessions: make(map[string]*codexWebsocketSession)}
				const sessionID = "active-cause-disconnect"
				disconnect := executor.UpstreamDisconnectChan(sessionID)
				ctx, cancel := context.WithCancel(t.Context())
				defer cancel()
				notified := make(chan error, 1)
				go func() {
					select {
					case cause := <-disconnect:
						// Match the production downstream-disconnect callback.
						if !completed {
							cancel()
						}
						notified <- cause
					case <-ctx.Done():
					}
				}()
				req, opts := codexWebsocketRequest()
				opts.Metadata = map[string]any{cliproxyexecutor.ExecutionSessionMetadataKey: sessionID}
				auth := codexTestAuth(server.URL)
				auth.ID = "stream-cause-usage/" + t.Name()
				result, streamErr := executor.ExecuteStream(ctx, auth, req, opts)
				if buffering && !completed && result != nil {
					t.Fatal("bootstrap read failure must use the pre-output error-return branch")
				}
				if result != nil {
					if streamErr != nil {
						t.Fatalf("executor returned a stream alongside an error: %v", streamErr)
					}
					_, streamErr = drainChunks(result)
				} else if !buffering || completed {
					t.Fatalf("expected an opened stream: %v", streamErr)
				}
				if !completed && (streamErr == nil || statusCodeFromTestError(t, streamErr) != 502 || gjson.Get(streamErr.Error(), "error.code").String() != "upstream_stream_error") {
					t.Fatalf("disconnect cancellation replaced the typed execution failure: %v", streamErr)
				}
				select {
				case cause := <-notified:
					code := gjson.Get(cause.Error(), "error.code").String()
					if completed {
						if streamErr != nil || code != "" {
							t.Fatalf("completed stream became a read failure: stream=%v disconnect=%v", streamErr, cause)
						}
					} else if streamErr == nil || code != "upstream_stream_error" || !strings.Contains(cause.Error(), "unexpected EOF") {
						t.Fatalf("early disconnect notification lost typed cause: stream=%v disconnect=%v", streamErr, cause)
					}
				case <-time.After(3 * time.Second):
					t.Fatal("missing upstream disconnect notification")
				}
				select {
				case record := <-plugin.records:
					if record.AuthID != auth.ID {
						t.Fatalf("usage record belongs to another execution: %s", record.AuthID)
					}
					if completed {
						if record.Failed {
							t.Fatalf("completed stream usage became a failure: %#v", record.Fail)
						}
					} else if !record.Failed || record.Fail.StatusCode != 502 || gjson.Get(record.Fail.Body, "error.code").String() != "upstream_stream_error" || !strings.Contains(record.Fail.Body, "unexpected EOF") {
						t.Fatalf("reporter lost the typed upstream cause after cancellation: %#v", record)
					}
				case <-time.After(3 * time.Second):
					t.Fatal("missing executor usage record")
				}
			})
		}
	}
}

func TestCodexStreamFailureDoesNotRotateOrCoolCredentialAndAllowsRetry(t *testing.T) {
	for _, buffering := range []bool{false, true} {
		t.Run(fmt.Sprint(buffering), func(t *testing.T) {
			var callsA, callsB atomic.Int32
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				first := r.Header.Get("Authorization") == "Bearer a-key"
				if !first {
					callsB.Add(1)
				} else {
					callsA.Add(1)
				}
				w.Header().Set("Content-Type", "text/event-stream")
				_, _ = fmt.Fprintf(w, "data: %s\n\n", codexCreatedEvent)
				if first && callsA.Load() == 1 {
					return
				}
				_, _ = fmt.Fprintf(w, "data: %s\n\n", codexCompletedEventBody)
			}))
			defer server.Close()
			cfg := codexBufferingConfig(buffering)
			manager := coreauth.NewManager(nil, nil, nil)
			manager.SetConfig(cfg)
			manager.SetRetryConfig(0, 0, 2)
			manager.RegisterExecutor(NewCodexExecutor(cfg))
			for i, name := range []string{"a", "b"} {
				id := "stream-causes-" + t.Name() + name
				registry.GetGlobalRegistry().RegisterClient(id, "codex", []*registry.ModelInfo{{ID: "gpt-5.6-terra"}})
				defer registry.GetGlobalRegistry().UnregisterClient(id)
				if _, err := manager.Register(t.Context(), &coreauth.Auth{ID: id, Provider: "codex", Status: coreauth.StatusActive,
					Attributes: map[string]string{"api_key": name + "-key", "base_url": server.URL, "priority": fmt.Sprint(100 - i)}}); err != nil {
					t.Fatal(err)
				}
			}
			for attempt := 0; attempt < 2; attempt++ {
				req, opts := codexTestRequest()
				result, err := manager.ExecuteStream(t.Context(), []string{"codex"}, req, opts)
				var body string
				if result != nil {
					body, err = drainChunks(result)
				}
				if attempt == 0 && (err == nil || statusCodeFromTestError(t, err) != 502) || attempt == 1 && (err != nil || !strings.Contains(body, "response.completed")) {
					t.Fatalf("retry attempt %d failed: %v %s", attempt, err, body)
				}
				for _, auth := range manager.List() {
					if auth.Unavailable || !auth.NextRetryAfter.IsZero() {
						t.Fatalf("stream failure cooled credential: %#v", auth)
					}
					for _, model := range auth.ModelStates {
						if model.Unavailable || !model.NextRetryAfter.IsZero() {
							t.Fatalf("stream failure cooled model: %#v", model)
						}
					}
				}
			}
			if callsA.Load() != 2 || callsB.Load() != 0 {
				t.Fatalf("request-scoped failure rotated accounts: a=%d b=%d", callsA.Load(), callsB.Load())
			}
		})
	}
}

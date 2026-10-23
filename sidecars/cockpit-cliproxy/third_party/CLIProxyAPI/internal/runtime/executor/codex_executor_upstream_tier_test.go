package executor

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

type captureCodexUpstreamTierPlugin struct {
	records chan usage.Record
}

func (p *captureCodexUpstreamTierPlugin) HandleUsage(_ context.Context, record usage.Record) {
	if !strings.HasPrefix(record.AuthID, "upstream-tier/") {
		return
	}
	select {
	case p.records <- record:
	default:
	}
}

// Compare usage metadata with the actual outbound HTTP or websocket body.
// All upstreams are loopback test servers; no external provider is contacted.
func TestCodexExecutorsCaptureFinalOutboundServiceTier(t *testing.T) {
	plugin := &captureCodexUpstreamTierPlugin{records: make(chan usage.Record, 64)}
	usage.RegisterPlugin(plugin)
	for _, transport := range []string{"http", "http-stream", "compact", "websocket", "websocket-stream"} {
		for _, authKind := range []string{"oauth", "api-key"} {
			for _, tier := range []string{"", "default", "priority"} {
				t.Run(transport+"/"+authKind+"/"+tier, func(t *testing.T) {
					authID := "upstream-tier/" + t.Name()
					captured := make(chan []byte, 1)
					completed := []byte(`{"type":"response.completed","response":{"id":"resp_tier","model":"gpt-5.4","status":"completed","service_tier":"default","output":[],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}`)
					upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
					server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
						if strings.HasPrefix(transport, "websocket") {
							conn, err := upgrader.Upgrade(w, r, nil)
							if err != nil {
								t.Errorf("upgrade: %v", err)
								return
							}
							defer conn.Close()
							_, body, err := conn.ReadMessage()
							if err != nil {
								t.Errorf("read message: %v", err)
								return
							}
							captured <- body
							if err := conn.WriteMessage(websocket.TextMessage, completed); err != nil {
								t.Errorf("write message: %v", err)
							}
							return
						}
						body, err := io.ReadAll(r.Body)
						if err != nil {
							t.Errorf("read HTTP body: %v", err)
							return
						}
						captured <- body
						if transport == "compact" {
							w.Header().Set("Content-Type", "application/json")
							_, _ = w.Write([]byte(`{"id":"resp_tier","object":"response.compaction","service_tier":"default","output":[],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}`))
						} else {
							w.Header().Set("Content-Type", "text/event-stream")
							_, _ = fmt.Fprintf(w, "data: %s\n\n", completed)
						}
					}))
					defer server.Close()
					cfg := &config.Config{
						SDKConfig: config.SDKConfig{DisableImageGeneration: config.DisableImageGenerationAll},
						Payload: config.PayloadConfig{Default: []config.PayloadRule{{
							Models: []config.PayloadModelRule{{Name: "*", Protocol: "codex"}, {Name: "*", Protocol: "openai-response"}},
							Params: map[string]any{"service_tier": "priority"},
						}}},
					}
					auth := &cliproxyauth.Auth{ID: authID, Provider: "codex", Attributes: map[string]string{"base_url": server.URL}}
					if authKind == "oauth" {
						auth.Metadata = map[string]any{"access_token": "fixture-token"}
					} else {
						auth.Attributes["api_key"] = "fixture-key"
					}
					body := `{"model":"gpt-5.4","input":"hello"`
					if tier != "" {
						body += `,"service_tier":"` + tier + `"`
					}
					body += `}`
					req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: []byte(body)}
					opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse}
					if transport == "compact" {
						opts.Alt = "responses/compact"
					}
					var executor interface {
						Execute(context.Context, *cliproxyauth.Auth, cliproxyexecutor.Request, cliproxyexecutor.Options) (cliproxyexecutor.Response, error)
						ExecuteStream(context.Context, *cliproxyauth.Auth, cliproxyexecutor.Request, cliproxyexecutor.Options) (*cliproxyexecutor.StreamResult, error)
					}
					if strings.HasPrefix(transport, "websocket") {
						executor = NewCodexWebsocketsExecutor(cfg)
					} else {
						executor = NewCodexExecutor(cfg)
					}
					ctx, cancel := context.WithTimeout(usage.WithServiceTier(context.Background(), "auto"), 5*time.Second)
					defer cancel()
					if strings.HasSuffix(transport, "stream") {
						result, err := executor.ExecuteStream(ctx, auth, req, opts)
						if err != nil {
							t.Fatalf("ExecuteStream: %v", err)
						}
						for chunk := range result.Chunks {
							if chunk.Err != nil {
								t.Fatalf("stream chunk: %v", chunk.Err)
							}
						}
					} else {
						if _, err := executor.Execute(ctx, auth, req, opts); err != nil {
							t.Fatalf("Execute: %v", err)
						}
					}
					var outbound []byte
					select {
					case outbound = <-captured:
					case <-ctx.Done():
						t.Fatal("no outbound payload")
					}
					actualTier := gjson.GetBytes(outbound, "service_tier").String()
					if tier == "" && actualTier != "priority" {
						t.Fatalf("missing client tier did not inject fast default: %s", outbound)
					}
					if tier == "default" && transport != "compact" && actualTier != "" {
						t.Fatalf("explicit default must be removed without fast reinjection: %s", outbound)
					}
					for {
						select {
						case record := <-plugin.records:
							if record.AuthID != authID {
								continue
							}
							if !record.UpstreamServiceTierObserved || record.UpstreamServiceTier != actualTier {
								t.Fatalf("record upstream=%q observed=%v, wire=%q", record.UpstreamServiceTier, record.UpstreamServiceTierObserved, actualTier)
							}
							if record.ServiceTier != "auto" || record.ResponseServiceTier != "default" {
								t.Fatalf("client/response tiers changed: %#v", record)
							}
							return
						case <-ctx.Done():
							t.Fatal("no usage record")
						}
					}
				})
			}
		}
	}
}

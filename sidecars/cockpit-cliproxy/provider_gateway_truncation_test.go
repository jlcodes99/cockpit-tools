package main

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

type providerGatewayEOFErrorReader struct{}

func (providerGatewayEOFErrorReader) Read([]byte) (int, error) { return 0, io.ErrUnexpectedEOF }

func TestProviderGatewayChatStreamRequiresCompleteOutput(t *testing.T) {
	gin.SetMode(gin.TestMode)
	const text = `data: {"id":"chat_1","choices":[{"index":0,"delta":{"content":"hello"}}]}` + "\n\n"
	const finish = `data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}` + "\n\n"
	for _, translated := range []bool{false, true} {
		for _, tc := range []struct {
			name, stream, terminal string
			readError, canceled    bool
		}{
			{name: "partial EOF", stream: text, terminal: "response.failed"},
			{name: "finish reason without DONE", stream: text + finish, terminal: "response.completed"},
			{name: "complete", stream: text + "data: [DONE]\n\n", terminal: "response.completed"},
			{name: "duplicate DONE", stream: text + finish + "data: [DONE]\n\ndata: [DONE]\n\n", terminal: "response.completed"},
			{name: "length finish", stream: text + `data: {"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}` + "\n\n", terminal: "response.incomplete"},
			{name: "read failure after finish", stream: text + finish, terminal: "response.failed", readError: true},
			{name: "upstream error after finish", stream: text + finish + `data: {"error":{"message":"failed","code":"server_error"}}` + "\n\n", terminal: "response.failed"},
			{name: "error event after finish", stream: text + finish + "event: error\n\ndata: [DONE]\n\n", terminal: "response.failed"},
			{name: "canceled after finish", stream: text + finish, canceled: true},
		} {
			name := tc.name
			if translated {
				name = "translated/" + name
			}
			t.Run(name, func(t *testing.T) {
				w := httptest.NewRecorder()
				c, _ := gin.CreateTestContext(w)
				ctx, cancel := context.WithCancel(context.Background())
				defer cancel()
				if tc.canceled {
					cancel()
				}
				c.Request = httptest.NewRequest("POST", "/v1/responses", nil).WithContext(ctx)
				var reader io.Reader = strings.NewReader(tc.stream)
				if tc.readError {
					reader = io.MultiReader(reader, providerGatewayEOFErrorReader{})
				}
				if translated {
					(&relayServer{}).writeProviderGatewayTranslatedChatStream(c, reader, "gpt-4o", []byte(`{}`), []byte(`{}`), sdktranslator.FormatOpenAIResponse, "")
				} else {
					(&relayServer{}).writeProviderGatewayChatStream(c, reader, "gpt-4o", []byte(`{}`), []byte(`{}`), false)
				}
				out := w.Body.String()
				if !strings.Contains(out, "hello") {
					t.Fatalf("partial output lost: %s", out)
				}
				for _, terminal := range []string{"response.failed", "response.completed", "response.incomplete"} {
					want := 0
					if terminal == tc.terminal {
						want = 1
					}
					if got := strings.Count(out, "event: "+terminal+"\n"); got != want {
						t.Fatalf("%s count = %d, want %d: %s", terminal, got, want, out)
					}
				}
			})
		}
	}
}

func TestProviderGatewayChatEOFRetainsUsageAndItemIdentity(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, translated := range []bool{false, true} {
		w := httptest.NewRecorder()
		c, _ := gin.CreateTestContext(w)
		c.Request = httptest.NewRequest("POST", "/v1/responses", nil)
		stream := strings.Join([]string{
			`data: {"id":"chat_1","choices":[{"index":0,"delta":{"content":"hello"}}]}`,
			`data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}`,
			`data: {"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}}`,
		}, "\n\n")
		if translated {
			(&relayServer{}).writeProviderGatewayTranslatedChatStream(c, strings.NewReader(stream), "gpt-4o", []byte(`{}`), []byte(`{}`), sdktranslator.FormatOpenAIResponse, "")
		} else {
			(&relayServer{}).writeProviderGatewayChatStream(c, strings.NewReader(stream), "gpt-4o", []byte(`{}`), []byte(`{}`), false)
		}
		assertProviderGatewayMessageIdentity(t, w.Body.String(), "hello")
		for _, line := range strings.Split(w.Body.String(), "\n") {
			if !strings.HasPrefix(line, "data:") {
				continue
			}
			event := gjson.Parse(strings.TrimSpace(strings.TrimPrefix(line, "data:")))
			if event.Get("type").String() == "response.completed" && (event.Get("response.usage.input_tokens").Int() != 10 || event.Get("response.usage.output_tokens").Int() != 2) {
				t.Fatalf("late usage lost during EOF finalization: %s", event.Raw)
			}
		}
	}
}

func TestProviderGatewayHTTPResponsesCompletesAtChatEOF(t *testing.T) {
	gin.SetMode(gin.TestMode)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/chat/completions" || r.Header.Get("Authorization") != "Bearer fixture-upstream" {
			t.Errorf("wrong upstream route or credential: %s", r.URL.Path)
		}
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = io.WriteString(w, "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n")
	}))
	defer upstream.Close()
	gateway := &providerGatewaySpec{BaseURL: upstream.URL, APIKey: "fixture-upstream", UpstreamModel: "fixture-model", UpstreamModels: []string{"fixture-model"}, WireAPI: "chat_completions"}
	spec := apiKeySpec{ID: "fixture-key", Key: "fixture-client", Enabled: true, ProviderGateway: gateway}
	m := &manifest{APIKeys: []apiKeySpec{spec}, ModelIDs: []string{"fixture-model"}, apiKeyByValue: map[string]*apiKeySpec{"fixture-client": &spec}}
	router := (&relayServer{runtime: &fakeRuntime{}, cfg: &config.Config{}, manifest: m, policy: &requestPolicy{manifest: m}}).router()
	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"fixture-model","input":"hello","stream":true}`))
	req.Header.Set("Authorization", "Bearer fixture-client")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	if w.Code != http.StatusOK || strings.Count(w.Body.String(), "event: response.completed\n") != 1 || strings.Contains(w.Body.String(), "response.failed") {
		t.Fatalf("EOF was not completed through the real gateway route: status=%d body=%s", w.Code, w.Body.String())
	}
	assertProviderGatewayMessageIdentity(t, w.Body.String(), "hello")
}

func TestProviderGatewayChatEOFFinalizesToolArguments(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, translated := range []bool{false, true} {
		w := httptest.NewRecorder()
		c, _ := gin.CreateTestContext(w)
		c.Request = httptest.NewRequest("POST", "/v1/responses", nil)
		stream := strings.Join([]string{
			`data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"lookup","arguments":"{\"x\":"}}]}}]}`,
			`data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]}}]}`,
			`data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}`,
		}, "\n\n")
		if translated {
			(&relayServer{}).writeProviderGatewayTranslatedChatStream(c, strings.NewReader(stream), "fixture-model", []byte(`{}`), []byte(`{}`), sdktranslator.FormatOpenAIResponse, "")
		} else {
			(&relayServer{}).writeProviderGatewayChatStream(c, strings.NewReader(stream), "fixture-model", []byte(`{}`), []byte(`{}`), false)
		}
		if strings.Count(w.Body.String(), "event: response.completed\n") != 1 || strings.Contains(w.Body.String(), "response.failed") {
			t.Fatalf("tool-call EOF did not complete: %s", w.Body.String())
		}
		for _, line := range strings.Split(w.Body.String(), "\n") {
			if !strings.HasPrefix(line, "data:") {
				continue
			}
			event := gjson.Parse(strings.TrimSpace(strings.TrimPrefix(line, "data:")))
			if event.Get("type").String() == "response.completed" {
				item := event.Get("response.output.0")
				if item.Get("call_id").String() != "call_1" || item.Get("arguments").String() != `{"x":1}` {
					t.Fatalf("tool call or arguments changed during EOF finalization: %s", item.Raw)
				}
			}
		}
	}
}

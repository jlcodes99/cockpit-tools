package main

import (
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
)

func TestProviderGatewayChatStreamFailsWhenNothingTranslates(t *testing.T) {
	// 上游每个 chunk 都不满足 chat→responses 转换器的形状门控（object 不是
	// chat.completion.chunk），随后正常 [DONE]：现状是 doneSeen=true 且
	// converted_event_count=0，客户端收到 200 + 空 SSE 体，完全无法诊断。
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Request = httptest.NewRequest("POST", "/v1/responses", nil)
	stream := "data: {\"object\":\"chat.completion\",\"choices\":[]}\n\ndata: [DONE]\n\n"
	(&relayServer{}).writeProviderGatewayChatStream(c, strings.NewReader(stream), "gpt-6.1-sol", []byte(`{}`), []byte(`{}`), false)
	out := w.Body.String()
	if failed := strings.Count(out, "event: response.failed\n"); failed != 1 {
		t.Fatalf("expected exactly one terminal response.failed for a stream with zero translated events, got %d: %s", failed, out)
	}
	if strings.Contains(out, "event: response.completed\n") {
		t.Fatalf("must not emit response.completed when nothing translated: %s", out)
	}
}

func TestProviderGatewayChatStreamRequiresDone(t *testing.T) {
	for _, tc := range []struct {
		name, suffix string
		failed       bool
	}{
		{"partial EOF", "", true},
		{"finish reason without DONE", "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n", true},
		{"complete", "data: [DONE]\n\n", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			w := httptest.NewRecorder()
			c, _ := gin.CreateTestContext(w)
			c.Request = httptest.NewRequest("POST", "/v1/responses", nil)
			stream := "data: {\"id\":\"chat_1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"}}]}\n\n" + tc.suffix
			(&relayServer{}).writeProviderGatewayChatStream(c, strings.NewReader(stream), "gpt-4o", []byte(`{}`), []byte(`{}`), false)
			out := w.Body.String()
			if !strings.Contains(out, "hello") {
				t.Fatalf("partial output lost: %s", out)
			}
			failed := strings.Count(out, "event: response.failed\n")
			completed := strings.Count(out, "event: response.completed\n")
			if tc.failed && (failed != 1 || completed != 0) || !tc.failed && (failed != 0 || completed != 1) {
				t.Fatalf("unexpected terminal events: failed=%d completed=%d: %s", failed, completed, out)
			}
		})
	}
}

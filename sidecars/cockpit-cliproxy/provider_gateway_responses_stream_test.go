package main

import (
	"bufio"
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func TestProviderGatewayResponsesStreamDeliversEventBeforeUpstreamCompletes(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, tc := range []struct{ name, newline string }{
		{"LF", "\n"},
		{"CRLF", "\r\n"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			firstEvent := "event: response.created" + tc.newline +
				`data: {"type":"response.created"}` + tc.newline + tc.newline
			lastEvent := "event: response.completed" + tc.newline +
				`data: {"type":"response.completed"}` + tc.newline + tc.newline
			release := make(chan struct{})
			var releaseOnce sync.Once
			finishUpstream := func() { releaseOnce.Do(func() { close(release) }) }
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.Header().Set("Content-Type", "text/event-stream")
				_, _ = io.WriteString(w, firstEvent)
				w.(http.Flusher).Flush()
				// Keep the response open until the client has received the first event.
				select {
				case <-release:
					_, _ = io.WriteString(w, lastEvent)
				case <-r.Context().Done():
				}
			}))
			defer upstream.Close()
			gateway := &providerGatewaySpec{
				BaseURL: upstream.URL, APIKey: "test-key", WireAPI: "responses",
				UpstreamModel: "test-model", UpstreamModels: []string{"test-model"},
			}
			body := []byte(`{"model":"test-model","input":"hello","stream":true}`)
			downstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				c, _ := gin.CreateTestContext(w)
				c.Request = r
				(&relayServer{}).handleProviderGatewayRequest(c, gateway, body, "test-model", sdktranslator.FormatOpenAIResponse, "")
			}))
			defer downstream.Close()
			defer finishUpstream()
			ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
			defer cancel()
			req, err := http.NewRequestWithContext(ctx, http.MethodPost, downstream.URL, strings.NewReader(string(body)))
			if err != nil {
				t.Fatal(err)
			}
			resp, err := downstream.Client().Do(req)
			if err != nil {
				t.Fatalf("response headers were not delivered while upstream remained open: %v", err)
			}
			defer resp.Body.Close()
			if resp.StatusCode != http.StatusOK {
				t.Fatalf("status = %d, want 200", resp.StatusCode)
			}
			reader := bufio.NewReader(resp.Body)
			var received strings.Builder
			for {
				line, err := reader.ReadString('\n')
				if err != nil {
					t.Fatalf("first event was not delivered while upstream remained open: %v", err)
				}
				received.WriteString(line)
				if strings.TrimRight(line, "\r\n") == "" {
					break
				}
			}
			if got := received.String(); got != firstEvent {
				t.Fatalf("first event = %q, want %q", got, firstEvent)
			}
			finishUpstream()
			tail, err := io.ReadAll(reader)
			if err != nil {
				t.Fatal(err)
			}
			if string(tail) != lastEvent {
				t.Fatalf("terminal event = %q, want %q", tail, lastEvent)
			}
		})
	}
}

func TestProviderGatewayResponsesStreamFlushesTrailingDataAtEOF(t *testing.T) {
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	body := "data: [DONE]"
	(&relayServer{}).writeProviderGatewayResponsesStream(c, strings.NewReader(body), false)
	if !w.Flushed || w.Body.String() != body {
		t.Fatalf("trailing data not flushed intact: flushed=%t body=%q", w.Flushed, w.Body.String())
	}
}

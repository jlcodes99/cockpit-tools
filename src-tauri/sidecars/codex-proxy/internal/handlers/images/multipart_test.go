package images

import (
	"bytes"
	"context"
	"io"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

type flushRecorder struct {
	*httptest.ResponseRecorder
	flushCount int
}

func (f *flushRecorder) Flush() {
	f.flushCount++
}

type brokenPipeWriter struct{}

func (brokenPipeWriter) Header() http.Header {
	return http.Header{}
}

func (brokenPipeWriter) Write([]byte) (int, error) {
	return 0, io.ErrClosedPipe
}

func (brokenPipeWriter) WriteHeader(statusCode int) {}

func (brokenPipeWriter) Flush() {}

func TestExtractOperation(t *testing.T) {
	tests := []struct {
		path string
		want string
	}{
		{path: "/v1/images/generations", want: operationGenerations},
		{path: "/prefix/v1/images/generations", want: operationGenerations},
		{path: "/v1/images/edits", want: operationEdits},
		{path: "/prefix/v1/images/edits", want: operationEdits},
		{path: "/v1/images/variations", want: operationVariations},
		{path: "/v1/images/unknown", want: ""},
	}

	for _, tt := range tests {
		if got := extractOperation(tt.path); got != tt.want {
			t.Fatalf("extractOperation(%q) = %q, want %q", tt.path, got, tt.want)
		}
	}
}

func TestPassthroughStreamingResponseClientDisconnect(t *testing.T) {
	gin.SetMode(gin.TestMode)
	c, _ := gin.CreateTestContext(brokenPipeWriter{})
	resp := &http.Response{
		StatusCode: http.StatusOK,
		Header:     http.Header{"Content-Type": []string{"text/event-stream"}},
		Body:       io.NopCloser(strings.NewReader("event: ping\ndata: hello\n\n")),
	}

	err := passthroughStreamingResponse(c, resp)
	if err != context.Canceled {
		t.Fatalf("expected context.Canceled, got %v", err)
	}
}

func TestBuildOperationRequest_JSONEdits(t *testing.T) {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	body := []byte(`{"model":"image-default","prompt":"add sparkles","stream":true}`)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/images/edits", bytes.NewReader(body))
	c.Request.Header.Set("Content-Type", "application/json")

	upstream := &config.UpstreamConfig{
		ServiceType: "openai",
		ModelMapping: map[string]string{
			"image-default": "gpt-image-2",
		},
	}
	req, err := buildOperationRequest(c, upstream, "https://api.openai.com", "sk-test", body, "image-default", operationEdits, "application/json")
	if err != nil {
		t.Fatalf("buildOperationRequest() error = %v", err)
	}
	if req.URL.String() != "https://api.openai.com/v1/images/edits" {
		t.Fatalf("unexpected url: %s", req.URL.String())
	}
	if req.Header.Get("Content-Type") != "application/json" {
		t.Fatalf("unexpected content type: %s", req.Header.Get("Content-Type"))
	}

	requestBody, err := io.ReadAll(req.Body)
	if err != nil {
		t.Fatalf("read request body: %v", err)
	}
	if !strings.Contains(string(requestBody), `"model":"gpt-image-2"`) {
		t.Fatalf("model mapping was not applied: %s", string(requestBody))
	}
}

func TestBuildOperationRequest_MultipartEdits(t *testing.T) {
	gin.SetMode(gin.TestMode)
	body := &bytes.Buffer{}
	writer := multipart.NewWriter(body)
	if err := writer.WriteField("model", "image-default"); err != nil {
		t.Fatalf("write model: %v", err)
	}
	if err := writer.WriteField("prompt", "add sparkles"); err != nil {
		t.Fatalf("write prompt: %v", err)
	}
	part, err := writer.CreateFormFile("image[]", "input.png")
	if err != nil {
		t.Fatalf("create image part: %v", err)
	}
	if _, err := part.Write([]byte("png-data")); err != nil {
		t.Fatalf("write image part: %v", err)
	}
	if err := writer.Close(); err != nil {
		t.Fatalf("close writer: %v", err)
	}

	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/images/edits", bytes.NewReader(body.Bytes()))
	contentType := writer.FormDataContentType()
	c.Request.Header.Set("Content-Type", contentType)

	upstream := &config.UpstreamConfig{
		ServiceType: "openai",
		ModelMapping: map[string]string{
			"image-default": "gpt-image-2",
		},
	}
	req, err := buildOperationRequest(c, upstream, "https://api.openai.com#", "sk-test", body.Bytes(), "image-default", operationEdits, contentType)
	if err != nil {
		t.Fatalf("buildOperationRequest() error = %v", err)
	}
	if req.URL.String() != "https://api.openai.com/images/edits" {
		t.Fatalf("unexpected url: %s", req.URL.String())
	}
	if !strings.HasPrefix(req.Header.Get("Content-Type"), "multipart/form-data; boundary=") {
		t.Fatalf("unexpected content type: %s", req.Header.Get("Content-Type"))
	}

	requestBody, err := io.ReadAll(req.Body)
	if err != nil {
		t.Fatalf("read request body: %v", err)
	}
	if !strings.Contains(string(requestBody), "gpt-image-2") {
		t.Fatalf("model mapping was not applied: %s", string(requestBody))
	}
	if !strings.Contains(string(requestBody), "png-data") {
		t.Fatalf("file part was not preserved: %s", string(requestBody))
	}
}

func TestBuildOperationRequest_VariationsURL(t *testing.T) {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	body := []byte(`{"n":1,"size":"1024x1024"}`)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/images/variations", bytes.NewReader(body))
	c.Request.Header.Set("Content-Type", "application/json")

	upstream := &config.UpstreamConfig{ServiceType: "openai"}
	req, err := buildOperationRequest(c, upstream, "https://api.openai.com", "sk-test", body, "", operationVariations, "application/json")
	if err != nil {
		t.Fatalf("buildOperationRequest() error = %v", err)
	}
	if req.URL.String() != "https://api.openai.com/v1/images/variations" {
		t.Fatalf("unexpected url: %s", req.URL.String())
	}
}

func TestPassthroughStreamingResponseFlushes(t *testing.T) {
	gin.SetMode(gin.TestMode)
	recorder := &flushRecorder{ResponseRecorder: httptest.NewRecorder()}
	c, _ := gin.CreateTestContext(recorder)
	resp := &http.Response{
		StatusCode: http.StatusOK,
		Header:     http.Header{"Content-Type": []string{"text/event-stream"}},
		Body:       io.NopCloser(strings.NewReader("event: ping\ndata: hello\n\n")),
	}

	if err := passthroughStreamingResponse(c, resp); err != nil {
		t.Fatalf("passthroughStreamingResponse() error = %v", err)
	}
	if recorder.flushCount == 0 {
		t.Fatal("expected streaming response to flush at least once")
	}
	if body := recorder.Body.String(); body != "event: ping\ndata: hello\n\n" {
		t.Fatalf("unexpected body: %q", body)
	}
}

func TestBuildJSONRequestBody_NullBodyDoesNotPanic(t *testing.T) {
	body, contentType, err := buildJSONRequestBody([]byte("null"), "gpt-image-2", "gpt-image-2", operationEdits)
	if err != nil {
		t.Fatalf("buildJSONRequestBody() error = %v", err)
	}
	if contentType != "application/json" {
		t.Fatalf("unexpected content type: %s", contentType)
	}
	if string(body) != `{"model":"gpt-image-2"}` {
		t.Fatalf("unexpected request body: %s", string(body))
	}
}

func TestBuildOperationRequest_AddsDefaultModelToMultipart(t *testing.T) {
	gin.SetMode(gin.TestMode)
	body := &bytes.Buffer{}
	writer := multipart.NewWriter(body)
	if err := writer.WriteField("prompt", "add sparkles"); err != nil {
		t.Fatalf("write prompt: %v", err)
	}
	if err := writer.Close(); err != nil {
		t.Fatalf("close writer: %v", err)
	}

	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/images/edits", bytes.NewReader(body.Bytes()))
	contentType := writer.FormDataContentType()
	c.Request.Header.Set("Content-Type", contentType)

	upstream := &config.UpstreamConfig{ServiceType: "openai"}
	req, err := buildOperationRequest(c, upstream, "https://api.openai.com", "sk-test", body.Bytes(), "gpt-image-2", operationEdits, contentType)
	if err != nil {
		t.Fatalf("buildOperationRequest() error = %v", err)
	}
	requestBody, err := io.ReadAll(req.Body)
	if err != nil {
		t.Fatalf("read request body: %v", err)
	}
	if !strings.Contains(string(requestBody), "gpt-image-2") {
		t.Fatalf("default model was not injected into multipart body: %s", string(requestBody))
	}
}

func TestBuildOperationRequest_PreservesQueryString(t *testing.T) {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	body := []byte(`{"model":"image-default","prompt":"add sparkles"}`)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/images/edits?stream=true&foo=bar", bytes.NewReader(body))
	c.Request.Header.Set("Content-Type", "application/json")

	upstream := &config.UpstreamConfig{
		ServiceType: "openai",
		ModelMapping: map[string]string{
			"image-default": "gpt-image-2",
		},
	}
	req, err := buildOperationRequest(c, upstream, "https://api.openai.com", "sk-test", body, "image-default", operationEdits, "application/json")
	if err != nil {
		t.Fatalf("buildOperationRequest() error = %v", err)
	}
	if req.URL.RawQuery != "stream=true&foo=bar" {
		t.Fatalf("unexpected raw query: %s", req.URL.RawQuery)
	}
}

func TestExtractImagesModelAndStream(t *testing.T) {
	jsonBody := []byte(`{"model":"gpt-image-2","stream":true}`)
	if got := extractImagesModel(jsonBody, "application/json"); got != "gpt-image-2" {
		t.Fatalf("extractImagesModel json = %q", got)
	}

	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/images/edits?stream=true", bytes.NewReader(jsonBody))
	if !isImagesStreamRequest(c, jsonBody, "application/json") {
		t.Fatal("expected JSON stream request")
	}

	body := &bytes.Buffer{}
	writer := multipart.NewWriter(body)
	_ = writer.WriteField("model", "gpt-image-2")
	_ = writer.WriteField("stream", "true")
	_ = writer.Close()
	contentType := writer.FormDataContentType()
	if got := extractImagesModel(body.Bytes(), contentType); got != "gpt-image-2" {
		t.Fatalf("extractImagesModel multipart = %q", got)
	}
	if !isImagesStreamRequest(c, body.Bytes(), contentType) {
		t.Fatal("expected multipart stream request")
	}
}

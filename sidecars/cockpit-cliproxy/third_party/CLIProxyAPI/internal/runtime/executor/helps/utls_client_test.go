package helps

import (
	"context"
	"io"
	"net/http"
	"strings"
	"testing"
)

type utlsClientRoundTripFunc func(*http.Request) (*http.Response, error)

func (f utlsClientRoundTripFunc) RoundTrip(req *http.Request) (*http.Response, error) {
	return f(req)
}

func TestNewUtlsHTTPClientUsesContextRoundTripperForChatGPT(t *testing.T) {
	t.Parallel()

	called := false
	ctx := context.WithValue(context.Background(), "cliproxy.roundtripper", utlsClientRoundTripFunc(func(req *http.Request) (*http.Response, error) {
		called = true
		if req.URL.Hostname() != "chatgpt.com" {
			t.Fatalf("hostname = %q, want chatgpt.com", req.URL.Hostname())
		}
		return &http.Response{
			StatusCode: http.StatusOK,
			Header:     make(http.Header),
			Body:       io.NopCloser(strings.NewReader("{}")),
			Request:    req,
		}, nil
	}))

	client := NewUtlsHTTPClient(ctx, nil, nil, 0)
	resp, err := client.Get("https://chatgpt.com/backend-api/codex/responses")
	if err != nil {
		t.Fatalf("client.Get returned error: %v", err)
	}
	if errClose := resp.Body.Close(); errClose != nil {
		t.Fatalf("response body close returned error: %v", errClose)
	}
	if !called {
		t.Fatal("expected context RoundTripper to handle ChatGPT request")
	}
}

func TestFallbackRoundTripperUsesStandardTransportForChatGPT(t *testing.T) {
	t.Parallel()

	utlsCalled := false
	standardCalled := false
	roundTripper := &fallbackRoundTripper{
		utls: utlsClientRoundTripFunc(func(req *http.Request) (*http.Response, error) {
			utlsCalled = true
			return nil, nil
		}),
		fallback: utlsClientRoundTripFunc(func(req *http.Request) (*http.Response, error) {
			standardCalled = true
			return &http.Response{
				StatusCode: http.StatusOK,
				Header:     make(http.Header),
				Body:       io.NopCloser(strings.NewReader("{}")),
				Request:    req,
			}, nil
		}),
	}
	req, err := http.NewRequest(http.MethodGet, "https://chatgpt.com/backend-api/codex/responses", nil)
	if err != nil {
		t.Fatalf("http.NewRequest returned error: %v", err)
	}
	resp, err := roundTripper.RoundTrip(req)
	if err != nil {
		t.Fatalf("RoundTrip returned error: %v", err)
	}
	if errClose := resp.Body.Close(); errClose != nil {
		t.Fatalf("response body close returned error: %v", errClose)
	}
	if utlsCalled {
		t.Fatal("chatgpt.com unexpectedly used the uTLS transport")
	}
	if !standardCalled {
		t.Fatal("chatgpt.com did not use the standard HTTP transport")
	}
}

func TestFallbackRoundTripperUsesUtlsForAnthropic(t *testing.T) {
	t.Parallel()

	utlsCalled := false
	standardCalled := false
	roundTripper := &fallbackRoundTripper{
		utls: utlsClientRoundTripFunc(func(req *http.Request) (*http.Response, error) {
			utlsCalled = true
			return &http.Response{
				StatusCode: http.StatusOK,
				Header:     make(http.Header),
				Body:       io.NopCloser(strings.NewReader("{}")),
				Request:    req,
			}, nil
		}),
		fallback: utlsClientRoundTripFunc(func(req *http.Request) (*http.Response, error) {
			standardCalled = true
			return nil, nil
		}),
	}
	req, err := http.NewRequest(http.MethodGet, "https://api.anthropic.com/v1/messages", nil)
	if err != nil {
		t.Fatalf("http.NewRequest returned error: %v", err)
	}
	resp, err := roundTripper.RoundTrip(req)
	if err != nil {
		t.Fatalf("RoundTrip returned error: %v", err)
	}
	if errClose := resp.Body.Close(); errClose != nil {
		t.Fatalf("response body close returned error: %v", errClose)
	}
	if !utlsCalled {
		t.Fatal("api.anthropic.com did not use the uTLS transport")
	}
	if standardCalled {
		t.Fatal("api.anthropic.com unexpectedly used the standard HTTP transport")
	}
}

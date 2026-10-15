package helps

import (
	"encoding/binary"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	tls "github.com/refraction-networking/utls"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

// Exercise the actual provider dispatch and CONNECT transport without contacting
// Anthropic, installing a root certificate, or using an external proxy.
func TestClaudeCodeTLSClientHelloThroughLoopbackCONNECT(t *testing.T) {
	type capture struct {
		host   string
		record []byte
		err    error
	}
	captured := make(chan capture, 1)
	proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		if req.Method != http.MethodConnect || req.Host != "api.anthropic.com:443" {
			http.Error(w, "unexpected CONNECT target", http.StatusBadRequest)
			captured <- capture{host: req.Host}
			return
		}
		conn, buffered, errHijack := w.(http.Hijacker).Hijack()
		if errHijack != nil {
			captured <- capture{err: errHijack}
			return
		}
		defer func() { _ = conn.Close() }()
		if errDeadline := conn.SetDeadline(time.Now().Add(3 * time.Second)); errDeadline != nil {
			captured <- capture{err: errDeadline}
			return
		}
		if _, errWrite := buffered.WriteString("HTTP/1.1 200 Connection Established\r\n\r\n"); errWrite != nil {
			captured <- capture{err: errWrite}
			return
		}
		if errFlush := buffered.Flush(); errFlush != nil {
			captured <- capture{err: errFlush}
			return
		}
		header := make([]byte, 5)
		if _, errRead := io.ReadFull(buffered, header); errRead != nil {
			captured <- capture{err: errRead}
			return
		}
		payload := make([]byte, binary.BigEndian.Uint16(header[3:5]))
		_, errRead := io.ReadFull(buffered, payload)
		captured <- capture{host: req.Host, record: append(header, payload...), err: errRead}
		// Close after capture; the transport must report a failed TLS handshake.
	}))
	defer proxy.Close()

	client := NewUtlsHTTPClient(t.Context(), nil, &cliproxyauth.Auth{ProxyURL: proxy.URL}, 3*time.Second)
	req, errRequest := http.NewRequestWithContext(t.Context(), http.MethodPost,
		"https://api.anthropic.com/v1/messages", strings.NewReader(`{"model":"fixture","messages":[]}`))
	if errRequest != nil {
		t.Fatal(errRequest)
	}
	response, errDo := client.Do(req)
	if response != nil && response.Body != nil {
		_ = response.Body.Close()
	}
	if errDo == nil {
		t.Fatal("capture-only proxy unexpectedly completed TLS")
	}
	select {
	case got := <-captured:
		if got.err != nil {
			t.Fatal(got.err)
		}
		if got.host != "api.anthropic.com:443" {
			t.Fatalf("CONNECT target = %q", got.host)
		}
		spec, errFingerprint := (&tls.Fingerprinter{}).FingerprintClientHello(got.record)
		if errFingerprint != nil {
			t.Fatal(errFingerprint)
		}
		summary := summarizeClaudeCodeClientHelloSpec(t, spec)
		if summary.JA3MD5 != "d871d02cecbde59abbf8f4806134addf" {
			t.Fatalf("production CONNECT fingerprint = %s (%s)", summary.JA3MD5, summary.JA3)
		}
		if len(summary.ALPN) != 1 || summary.ALPN[0] != "http/1.1" {
			t.Fatalf("ALPN = %v", summary.ALPN)
		}
		t.Logf("loopback CONNECT captured %d TLS bytes, JA3=%s, ALPN=%v", len(got.record), summary.JA3MD5, summary.ALPN)
	case <-time.After(4 * time.Second):
		t.Fatal("production transport did not send a ClientHello through loopback CONNECT")
	}
}

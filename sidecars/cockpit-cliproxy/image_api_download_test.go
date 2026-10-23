package main

import (
	"bytes"
	"context"
	"crypto/x509"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"errors"
	"hash/crc32"
	"image"
	"image/gif"
	"image/jpeg"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

const imageAPITestPNGBase64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="

func imageAPITestPNG(t *testing.T) []byte {
	t.Helper()
	data, err := base64.StdEncoding.DecodeString(imageAPITestPNGBase64)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func imageAPITestDownloadPolicy(t *testing.T, target string) *imageDownloadPolicy {
	t.Helper()
	u, err := url.Parse(target)
	if err != nil {
		t.Fatal(err)
	}
	return &imageDownloadPolicy{
		lookup: func(context.Context, string) ([]net.IPAddr, error) {
			return []net.IPAddr{{IP: net.ParseIP("8.8.8.8")}}, nil
		},
		dial: func(ctx context.Context, network, address string) (net.Conn, error) {
			if !strings.HasPrefix(address, "8.8.8.8:") {
				t.Errorf("download was not pinned to a public IP: %s", address)
			}
			return (&net.Dialer{}).DialContext(ctx, network, u.Host)
		},
	}
}

func TestImageDownloadRejectsNonPublicDestinations(t *testing.T) {
	for _, destination := range []string{"localhost", "LOCALHOST.", "x.localhost", "127.0.0.1", "10.0.0.1", "169.254.169.254", "100.100.100.200", "172.16.1.2", "192.168.1.1", "0.0.0.0", "224.0.0.1", "198.18.0.1", "192.0.2.1", "[::1]", "[::ffff:127.0.0.1]", "[fe80::1]", "[fc00::1]", "[ff02::1]", "[64:ff9b::a00:1]", "[2002:a00:1::1]"} {
		t.Run(destination, func(t *testing.T) {
			var dials atomic.Int32
			policy := &imageDownloadPolicy{dial: func(context.Context, string, string) (net.Conn, error) { dials.Add(1); return nil, io.EOF }}
			client, err := imageDownloadClient(&imageAPIUpstreamSpec{}, policy)
			if err != nil {
				t.Fatal(err)
			}
			_, _, err = imageAPIUpload(context.Background(), client, "http://"+destination+"/secret")
			if err == nil || dials.Load() != 0 {
				t.Fatalf("private destination accessed: %v dials=%d", err, dials.Load())
			}
		})
	}
	for _, ips := range [][]net.IPAddr{{{IP: net.ParseIP("127.0.0.1")}}, {{IP: net.ParseIP("8.8.8.8")}, {IP: net.ParseIP("10.0.0.1")}}, {}} {
		var dials atomic.Int32
		client, _ := imageDownloadClient(&imageAPIUpstreamSpec{}, &imageDownloadPolicy{
			lookup: func(context.Context, string) ([]net.IPAddr, error) { return ips, nil },
			dial:   func(context.Context, string, string) (net.Conn, error) { dials.Add(1); return nil, io.EOF },
		})
		if _, _, err := imageAPIUpload(context.Background(), client, "http://images.test/secret"); err == nil || dials.Load() != 0 {
			t.Fatalf("non-public DNS accepted: %v dials=%d", err, dials.Load())
		}
	}
}

func TestImageDownloadPinsDNSAndValidatesPixels(t *testing.T) {
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		if r.Host != "images.test" || r.Header.Get("Authorization") != "" {
			t.Errorf("download changed host or leaked credential: %s %v", r.Host, r.Header)
		}
		switch r.URL.Path {
		case "/redirect":
			http.Redirect(w, r, "http://127.0.0.1/secret", http.StatusFound)
		case "/spoof":
			w.Header().Set("Content-Type", "image/png")
			_, _ = io.WriteString(w, "private text")
		case "/truncated":
			_, _ = w.Write(imageAPITestPNG(t)[:40])
		case "/large":
			_, _ = io.Copy(w, io.LimitReader(&imageTestZeroReader{}, maxImageUploadBytes+1))
		default:
			w.Header().Set("Content-Type", "text/plain")
			_, _ = w.Write(imageAPITestPNG(t))
		}
	}))
	defer server.Close()
	policy := imageAPITestDownloadPolicy(t, server.URL)
	var lookups atomic.Int32
	policy.lookup = func(context.Context, string) ([]net.IPAddr, error) {
		if lookups.Add(1) == 1 {
			return []net.IPAddr{{IP: net.ParseIP("8.8.8.8")}}, nil
		}
		return []net.IPAddr{{IP: net.ParseIP("127.0.0.1")}}, nil
	}
	client, _ := imageDownloadClient(&imageAPIUpstreamSpec{}, policy)
	data, mime, err := imageAPIUpload(context.Background(), client, "http://images.test/valid")
	if err != nil || mime != "image/png" || !bytes.Equal(data, imageAPITestPNG(t)) || lookups.Load() != 1 {
		t.Fatalf("not pinned/decoded correctly: %s %v lookups=%d", mime, err, lookups.Load())
	}
	if _, _, err = imageAPIUpload(context.Background(), client, "http://images.test/rebound"); err == nil || requests.Load() != 1 {
		t.Fatal("later private DNS result reached server")
	}
	for _, path := range []string{"redirect", "spoof", "truncated", "large"} {
		client, _ := imageDownloadClient(&imageAPIUpstreamSpec{}, imageAPITestDownloadPolicy(t, server.URL))
		before := requests.Load()
		if _, _, err := imageAPIUpload(context.Background(), client, "http://images.test/"+path); err == nil || requests.Load() != before+1 {
			t.Fatalf("unsafe output %s accepted: %v requests=%d", path, err, requests.Load())
		}
	}
}

type imageTestZeroReader struct{}

func (*imageTestZeroReader) Read(p []byte) (int, error) { clear(p); return len(p), nil }

func TestImageDownloadHTTPSPreservesServerName(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.TLS.ServerName != "example.com" || r.Host != "example.com" {
			t.Errorf("host verification changed: %s %s", r.TLS.ServerName, r.Host)
		}
		_, _ = w.Write(imageAPITestPNG(t))
	}))
	defer server.Close()
	client, _ := imageDownloadClient(&imageAPIUpstreamSpec{}, imageAPITestDownloadPolicy(t, server.URL))
	transport := client.Transport.(*imageDownloadTransport)
	roots := x509.NewCertPool()
	roots.AddCert(server.Certificate())
	transport.base.TLSClientConfig = server.Client().Transport.(*http.Transport).TLSClientConfig.Clone()
	transport.base.TLSClientConfig.InsecureSkipVerify = false
	transport.base.TLSClientConfig.RootCAs = roots
	if _, _, err := imageAPIUpload(context.Background(), client, "https://example.com/image"); err != nil {
		t.Fatal(err)
	}
	if _, _, err := imageAPIUpload(context.Background(), client, "https://wrong.example/image"); err == nil {
		t.Fatal("wrong TLS hostname accepted")
	}
}

func TestImageDownloadProxyTunnelsOnlyVerifiedIP(t *testing.T) {
	imageServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Host != "images.test" || r.Header.Get("Authorization") != "" {
			t.Error("image host changed or credential leaked")
		}
		_, _ = w.Write(imageAPITestPNG(t))
	}))
	defer imageServer.Close()
	target, _ := url.Parse(imageServer.URL)
	var calls atomic.Int32
	proxyServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.Method != http.MethodConnect || r.Host != "8.8.8.8:80" {
			t.Errorf("proxy target was not pinned: %s %s", r.Method, r.Host)
			w.WriteHeader(400)
			return
		}
		remote, err := net.Dial("tcp", target.Host)
		if err != nil {
			t.Error(err)
			return
		}
		local, buffered, err := w.(http.Hijacker).Hijack()
		if err != nil {
			_ = remote.Close()
			t.Error(err)
			return
		}
		_, _ = buffered.WriteString("HTTP/1.1 200 Connection Established\r\n\r\n")
		_ = buffered.Flush()
		go func() { _, _ = io.Copy(remote, buffered); _ = remote.Close() }()
		_, _ = io.Copy(local, remote)
		_ = local.Close()
	}))
	defer proxyServer.Close()
	client, err := imageDownloadClient(&imageAPIUpstreamSpec{ProxyURL: proxyServer.URL}, &imageDownloadPolicy{
		lookup: func(context.Context, string) ([]net.IPAddr, error) {
			return []net.IPAddr{{IP: net.ParseIP("8.8.8.8")}}, nil
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := imageAPIUpload(context.Background(), client, "http://images.test/image"); err != nil {
		t.Fatal(err)
	}
	if _, _, err := imageAPIUpload(context.Background(), client, "http://127.0.0.1/secret"); err == nil || calls.Load() != 1 {
		t.Fatalf("private IP reached proxy: %v calls=%d", err, calls.Load())
	}
}

func TestImageAPIBlocksPrivateReferenceMaskAndOutput(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		_ = json.NewEncoder(w).Encode(map[string]any{"data": []any{map[string]any{"url": "http://127.0.0.1/secret"}}})
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
	for _, body := range []string{
		`{"model":"gpt-image-2.5","prompt":"edit","image":"http://127.0.0.1/secret"}`,
		`{"model":"gpt-image-2.5","prompt":"edit","image":"data:image/png;base64,` + imageAPITestPNGBase64 + `","mask":{"image_url":"http://169.254.169.254/secret"}}`,
		`{"model":"gpt-image-2.5","prompt":"edit","image":"data:image/png;base64,c2VjcmV0"}`,
	} {
		resp := imageAPITestRequest(s, imagesEditsPath, body)
		if resp.Code != 400 || calls.Load() != 0 {
			t.Fatalf("unsafe input submitted: %d calls=%d %s", resp.Code, calls.Load(), resp.Body.String())
		}
	}
	resp := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if resp.Code != 502 || calls.Load() != 1 {
		t.Fatalf("unsafe output accepted: %d calls=%d %s", resp.Code, calls.Load(), resp.Body.String())
	}
}

func TestImageAPIValidatesActualImageFormatsAndDataURL(t *testing.T) {
	formats := map[string][]byte{"png": imageAPITestPNG(t)}
	for _, format := range []string{"gif", "jpeg"} {
		var encoded bytes.Buffer
		var err error
		if format == "gif" {
			err = gif.Encode(&encoded, image.NewRGBA(image.Rect(0, 0, 1, 1)), nil)
		} else {
			err = jpeg.Encode(&encoded, image.NewRGBA(image.Rect(0, 0, 1, 1)), nil)
		}
		if err != nil {
			t.Fatal(err)
		}
		formats[format] = encoded.Bytes()
	}
	formats["webp"], _ = base64.StdEncoding.DecodeString("UklGRhwAAABXRUJQVlA4TA8AAAAvAAAAAAcQ/Y/+ByKi/wEA")
	for format, data := range formats {
		mime, err := validateImageAPIBytes(data)
		if err != nil || mime != "image/"+format {
			t.Fatalf("valid %s rejected: %s %v", format, mime, err)
		}
		// Claimed MIME must not affect actual image type.
		_, mime, err = imageAPIUpload(context.Background(), nil, "data:text/plain;base64,"+base64.StdEncoding.EncodeToString(data))
		if err != nil || mime != "image/"+format {
			t.Fatalf("data URL trusted its MIME: %s %v", mime, err)
		}
	}
	if _, err := validateImageAPIBytes([]byte("<svg xmlns='http://www.w3.org/2000/svg'></svg>")); err == nil {
		t.Fatal("non-raster image accepted")
	}
	bomb := append([]byte(nil), imageAPITestPNG(t)...)
	binary.BigEndian.PutUint32(bomb[16:20], 100000)
	binary.BigEndian.PutUint32(bomb[20:24], 100000)
	binary.BigEndian.PutUint32(bomb[29:33], crc32.ChecksumIEEE(bomb[12:29]))
	if _, err := validateImageAPIBytes(bomb); err == nil || !strings.Contains(err.Error(), "pixel limit") {
		t.Fatalf("large image dimensions were not rejected before decoding: %v", err)
	}
}

func TestImageDownloadDNSIsBoundedAndCancelled(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	client, _ := imageDownloadClient(&imageAPIUpstreamSpec{}, &imageDownloadPolicy{
		lookup: func(ctx context.Context, _ string) ([]net.IPAddr, error) {
			deadline, ok := ctx.Deadline()
			if !ok || time.Until(deadline) > 10*time.Second {
				t.Error("DNS lacks a bounded deadline")
			}
			<-ctx.Done()
			return nil, ctx.Err()
		},
		dial: func(context.Context, string, string) (net.Conn, error) {
			t.Error("cancelled DNS reached dial")
			return nil, io.EOF
		},
	})
	if _, _, err := imageAPIUpload(ctx, client, "http://images.test/image"); !errors.Is(err, context.Canceled) {
		t.Fatalf("DNS cancellation lost: %v", err)
	}
}

func TestImageDownloadSOCKSProxyUsesNumericTarget(t *testing.T) {
	imageServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { _, _ = w.Write(imageAPITestPNG(t)) }))
	defer imageServer.Close()
	target, _ := url.Parse(imageServer.URL)
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	done := make(chan struct{})
	go func() {
		defer close(done)
		conn, err := listener.Accept()
		if err != nil {
			return
		}
		defer conn.Close()
		_ = conn.SetDeadline(time.Now().Add(3 * time.Second))
		var greeting [2]byte
		if _, err = io.ReadFull(conn, greeting[:]); err != nil {
			t.Error(err)
			return
		}
		methods := make([]byte, int(greeting[1]))
		_, _ = io.ReadFull(conn, methods)
		_, _ = conn.Write([]byte{5, 0})
		var request [10]byte
		if _, err = io.ReadFull(conn, request[:]); err != nil {
			t.Error(err)
			return
		}
		if request[3] != 1 || !net.IP(request[4:8]).Equal(net.ParseIP("8.8.8.8")) || binary.BigEndian.Uint16(request[8:]) != 80 {
			t.Errorf("SOCKS target was not pinned IPv4: %v", request)
			return
		}
		remote, err := net.Dial("tcp", target.Host)
		if err != nil {
			t.Error(err)
			return
		}
		defer remote.Close()
		_, _ = conn.Write([]byte{5, 0, 0, 1, 127, 0, 0, 1, 0, 0})
		go func() { _, _ = io.Copy(remote, conn); _ = remote.Close() }()
		_, _ = io.Copy(conn, remote)
	}()
	client, err := imageDownloadClient(&imageAPIUpstreamSpec{ProxyURL: "socks5h://" + listener.Addr().String()}, &imageDownloadPolicy{
		lookup: func(context.Context, string) ([]net.IPAddr, error) {
			return []net.IPAddr{{IP: net.ParseIP("8.8.8.8")}}, nil
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := imageAPIUpload(context.Background(), client, "http://images.test/image"); err != nil {
		t.Fatal(err)
	}
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("SOCKS download did not close its connection")
	}
}

func TestImageAPIEditDownloadsPublicReferenceAndMask(t *testing.T) {
	var downloads atomic.Int32
	download := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		downloads.Add(1)
		if r.Header.Get("Authorization") != "" {
			t.Fatal("download received the API credential")
		}
		// The multipart part MIME is derived from bytes rather than this header.
		w.Header().Set("Content-Type", "text/plain")
		_, _ = w.Write(imageAPITestPNG(t))
	}))
	defer download.Close()
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if err := r.ParseMultipartForm(1 << 20); err != nil {
			t.Error(err)
			return
		}
		for _, name := range []string{"image[]", "mask"} {
			file, header, err := r.FormFile(name)
			if err != nil {
				t.Error(err)
				continue
			}
			data, _ := io.ReadAll(file)
			_ = file.Close()
			if header.Header.Get("Content-Type") != "image/png" || !bytes.Equal(data, imageAPITestPNG(t)) {
				t.Errorf("downloaded %s was not normalized: %v", name, header.Header)
			}
		}
		_, _ = io.WriteString(w, `{"data":[{"b64_json":"`+imageAPITestPNGBase64+`"}]}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
	s.imageDownloads = imageAPITestDownloadPolicy(t, download.URL)
	resp := imageAPITestRequest(s, imagesEditsPath, `{"model":"gpt-image-2.5","prompt":"edit","image":"http://images.test/reference","mask":{"image_url":"http://images.test/mask"}}`)
	if resp.Code != 200 || downloads.Load() != 2 {
		t.Fatalf("public reference/mask edit failed: %d downloads=%d %s", resp.Code, downloads.Load(), resp.Body.String())
	}
}

func TestImageDownloadRedirectsRevalidateEveryHop(t *testing.T) {
	for _, destination := range []string{"public", "private", "private-dns", "mixed-dns", "credentials", "scheme", "loop"} {
		t.Run(destination, func(t *testing.T) {
			var requests, privateRequests, lookups atomic.Int32
			private := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				privateRequests.Add(1)
				_, _ = w.Write(imageAPITestPNG(t))
			}))
			defer private.Close()
			public := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				requests.Add(1)
				if r.Header.Get("Authorization") != "" || r.Header.Get("Proxy-Authorization") != "" || r.Header.Get("Cookie") != "" {
					t.Error("redirect carried credentials")
				}
				if r.URL.Path == "/image" {
					if r.Host != "cdn.test" {
						t.Errorf("CDN Host changed: %s", r.Host)
					}
					_, _ = w.Write(imageAPITestPNG(t))
					return
				}
				target := "http://cdn.test/image"
				switch destination {
				case "private":
					target = private.URL + "/secret"
				case "private-dns":
					target = "http://internal.test/secret"
				case "credentials":
					target = "http://user:secret@cdn.test/image"
				case "scheme":
					target = "file:///secret"
				case "loop":
					target = "http://cdn.test/redirect"
				}
				http.Redirect(w, r, target, http.StatusFound)
			}))
			defer public.Close()
			policy := imageAPITestDownloadPolicy(t, public.URL)
			policy.lookup = func(_ context.Context, host string) ([]net.IPAddr, error) {
				lookups.Add(1)
				if host == "internal.test" {
					return []net.IPAddr{{IP: net.ParseIP("127.0.0.1")}}, nil
				}
				if host != "images.test" && host != "cdn.test" {
					t.Errorf("unexpected hostname lookup: %s", host)
				}
				if destination == "mixed-dns" && host == "cdn.test" {
					return []net.IPAddr{{IP: net.ParseIP("8.8.8.8")}, {IP: net.ParseIP("10.0.0.1")}}, nil
				}
				return []net.IPAddr{{IP: net.ParseIP("8.8.8.8")}}, nil
			}
			client, _ := imageDownloadClient(&imageAPIUpstreamSpec{}, policy)
			data, mime, err := imageAPIUpload(context.Background(), client, "http://images.test/redirect")
			if destination == "public" {
				if err != nil || mime != "image/png" || !bytes.Equal(data, imageAPITestPNG(t)) || requests.Load() != 2 || lookups.Load() != 2 {
					t.Fatalf("public CDN redirect failed: %v requests=%d lookups=%d", err, requests.Load(), lookups.Load())
				}
				return
			}
			wantRequests := int32(1)
			if destination == "loop" {
				wantRequests = 6 // Initial request plus at most five redirects.
				if err == nil || !strings.Contains(err.Error(), "redirect limit") {
					t.Fatalf("redirect loop limit lost: %v", err)
				}
			}
			if err == nil || requests.Load() != wantRequests || privateRequests.Load() != 0 {
				t.Fatalf("unsafe redirect was followed: %v public=%d private=%d", err, requests.Load(), privateRequests.Load())
			}
		})
	}
}

func TestImageDownloadRedirectClearsCredentialHeaders(t *testing.T) {
	client, _ := imageDownloadClient(&imageAPIUpstreamSpec{}, nil)
	req, _ := http.NewRequest(http.MethodGet, "http://cdn.test/image", nil)
	for _, name := range []string{"Authorization", "Proxy-Authorization", "Cookie"} {
		req.Header.Set(name, "must-not-follow")
	}
	if err := client.CheckRedirect(req, []*http.Request{{}}); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"Authorization", "Proxy-Authorization", "Cookie"} {
		if req.Header.Get(name) != "" {
			t.Fatalf("redirect retained %s", name)
		}
	}
}

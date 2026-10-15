package main

import (
	"bytes"
	"context"
	"fmt"
	"image"
	_ "image/gif"
	_ "image/jpeg"
	_ "image/png"
	"io"
	"net"
	"net/http"
	"net/netip"
	"strings"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/sdk/proxyutil"
	_ "golang.org/x/image/webp"
	"golang.org/x/net/proxy"
)

// Download destinations are untrusted, unlike the user-configured API endpoint.
// The resolver/dial hooks allow local tests without a production private-IP bypass.
type imageDownloadPolicy struct {
	lookup func(context.Context, string) ([]net.IPAddr, error)
	dial   func(context.Context, string, string) (net.Conn, error)
}

var imageNonPublicNetworks = []netip.Prefix{
	netip.MustParsePrefix("0.0.0.0/8"), netip.MustParsePrefix("10.0.0.0/8"),
	netip.MustParsePrefix("100.64.0.0/10"), netip.MustParsePrefix("127.0.0.0/8"),
	netip.MustParsePrefix("169.254.0.0/16"), netip.MustParsePrefix("172.16.0.0/12"),
	netip.MustParsePrefix("192.0.0.0/24"), netip.MustParsePrefix("192.0.2.0/24"),
	netip.MustParsePrefix("192.88.99.0/24"),
	netip.MustParsePrefix("192.168.0.0/16"), netip.MustParsePrefix("198.18.0.0/15"),
	netip.MustParsePrefix("198.51.100.0/24"), netip.MustParsePrefix("203.0.113.0/24"),
	netip.MustParsePrefix("224.0.0.0/3"), netip.MustParsePrefix("2001::/23"),
	netip.MustParsePrefix("2001:db8::/32"), netip.MustParsePrefix("2002::/16"),
	netip.MustParsePrefix("3fff::/20"),
}

func publicImageAddress(ip net.IP) bool {
	addr, ok := netip.AddrFromSlice(ip)
	if !ok {
		return false
	}
	addr = addr.Unmap()
	if !addr.IsGlobalUnicast() || addr.IsPrivate() || addr.IsLoopback() || addr.IsLinkLocalUnicast() {
		return false
	}
	// Exclude IPv6 translation/special-use ranges that can embed private IPv4.
	if addr.Is6() && !netip.MustParsePrefix("2000::/3").Contains(addr) {
		return false
	}
	for _, prefix := range imageNonPublicNetworks {
		if prefix.Contains(addr) {
			return false
		}
	}
	return true
}

type imageDownloadTransport struct {
	policy *imageDownloadPolicy
	base   *http.Transport
}

func imageDownloadClient(upstream *imageAPIUpstreamSpec, policy *imageDownloadPolicy) (*http.Client, error) {
	transport := proxyutil.NewDirectTransport()
	if strings.TrimSpace(upstream.ProxyURL) != "" {
		dialer, _, err := proxyutil.BuildDialer(upstream.ProxyURL)
		if err != nil {
			return nil, err
		}
		if dialer != nil {
			contextDialer, ok := dialer.(proxy.ContextDialer)
			if !ok {
				return nil, fmt.Errorf("image download proxy does not support cancellation")
			}
			// Tunnel to a verified numeric IP, never ask the proxy to resolve the
			// untrusted hostname. This applies to HTTP(S) and SOCKS proxies alike.
			transport.DialContext = contextDialer.DialContext
		}
	}
	return &http.Client{Transport: &imageDownloadTransport{policy: policy, base: transport},
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if len(via) > 5 {
				return fmt.Errorf("image download exceeds redirect limit")
			}
			if req.URL.Scheme != "https" && req.URL.Scheme != "http" || req.URL.User != nil {
				return fmt.Errorf("image redirect requires an HTTP URL without credentials")
			}
			req.Header.Del("Authorization")
			req.Header.Del("Proxy-Authorization")
			req.Header.Del("Cookie")
			// RoundTrip revalidates every hop and pins its DNS result independently.
			return nil
		}}, nil
}

func (t *imageDownloadTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	if req.URL.Scheme != "https" && req.URL.Scheme != "http" || req.URL.User != nil {
		return nil, fmt.Errorf("image download requires an HTTP URL without credentials")
	}
	host := req.URL.Hostname()
	if host == "" || strings.Contains(host, "%") || strings.EqualFold(strings.TrimSuffix(host, "."), "localhost") ||
		strings.HasSuffix(strings.ToLower(strings.TrimSuffix(host, ".")), ".localhost") {
		return nil, fmt.Errorf("image download destination is not public")
	}
	lookup := net.DefaultResolver.LookupIPAddr
	if t.policy != nil && t.policy.lookup != nil {
		lookup = t.policy.lookup
	}
	var addresses []net.IPAddr
	if ip := net.ParseIP(host); ip != nil {
		addresses = []net.IPAddr{{IP: ip}}
	} else {
		var err error
		lookupCtx, cancel := context.WithTimeout(req.Context(), 10*time.Second)
		addresses, err = lookup(lookupCtx, host)
		cancel()
		if err != nil {
			return nil, err
		}
	}
	if len(addresses) == 0 {
		return nil, fmt.Errorf("image download destination has no public address")
	}
	for _, address := range addresses {
		if address.Zone != "" || !publicImageAddress(address.IP) {
			return nil, fmt.Errorf("image download destination is not public")
		}
	}
	port := req.URL.Port()
	if port == "" {
		port = "80"
		if req.URL.Scheme == "https" {
			port = "443"
		}
	}
	address := net.JoinHostPort(addresses[0].IP.String(), port)
	transport := t.base.Clone()
	transport.DialTLSContext = nil
	transport.DialTLS = nil
	dial := transport.DialContext
	if t.policy != nil && t.policy.dial != nil {
		dial = t.policy.dial
	}
	if dial == nil {
		dial = (&net.Dialer{Timeout: 30 * time.Second}).DialContext
	}
	transport.DialContext = func(ctx context.Context, network, _ string) (net.Conn, error) {
		return dial(ctx, network, address)
	}
	// Keep the original URL for Host and TLS certificate verification. Only the
	// connection target changes, so a second DNS lookup cannot rebind it.
	resp, err := transport.RoundTrip(req)
	if err != nil {
		transport.CloseIdleConnections()
		return nil, err
	}
	resp.Body = &imageDownloadBody{ReadCloser: resp.Body, closeIdle: transport.CloseIdleConnections}
	return resp, nil
}

type imageDownloadBody struct {
	io.ReadCloser
	closeIdle func()
}

func (b *imageDownloadBody) Close() error {
	err := b.ReadCloser.Close()
	b.closeIdle()
	return err
}

func validateImageAPIBytes(data []byte) (string, error) {
	if len(data) == 0 || int64(len(data)) > maxImageUploadBytes {
		return "", fmt.Errorf("image is empty or exceeds size limit")
	}
	config, format, err := image.DecodeConfig(bytes.NewReader(data))
	if err != nil || config.Width <= 0 || config.Height <= 0 || int64(config.Width) > (25<<20)/int64(config.Height) {
		return "", fmt.Errorf("image has invalid format or exceeds pixel limit")
	}
	// DecodeConfig alone accepts truncated headers; decode verifies real pixels.
	if _, _, err = image.Decode(bytes.NewReader(data)); err != nil {
		return "", fmt.Errorf("image data is invalid: %w", err)
	}
	switch format {
	case "png", "jpeg", "gif", "webp":
		return "image/" + format, nil
	default:
		return "", fmt.Errorf("image format is unsupported")
	}
}

package httpclient

import (
	"context"
	"crypto/tls"
	"fmt"
	"log"
	"net"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"golang.org/x/net/proxy"
)

// ClientManager HTTP 客户端管理器
type ClientManager struct {
	mu      sync.RWMutex
	clients map[string]*http.Client
}

var globalManager = &ClientManager{
	clients: make(map[string]*http.Client),
}

// GetManager 获取全局客户端管理器
func GetManager() *ClientManager {
	return globalManager
}

// GetStandardClient 获取标准客户端（有超时，用于普通请求）
// 注意：启用自动压缩让Go处理gzip，配合请求头清理确保正确解压
// proxyURL: 可选的代理地址（支持 http/https/socks5 协议）
func (cm *ClientManager) GetStandardClient(timeout time.Duration, insecure bool, proxyURL ...string) *http.Client {
	return cm.getStandardClient(timeout, insecure, true, proxyURL...)
}

// NewStandardClient 获取标准客户端但不进入缓存（适用于临时代理等高变参数）
func (cm *ClientManager) NewStandardClient(timeout time.Duration, insecure bool, proxyURL ...string) *http.Client {
	return cm.getStandardClient(timeout, insecure, false, proxyURL...)
}

func (cm *ClientManager) getStandardClient(timeout time.Duration, insecure bool, useCache bool, proxyURL ...string) *http.Client {
	// 从配置获取响应头超时时间
	envConfig := config.NewEnvConfig()
	responseHeaderTimeout := time.Duration(envConfig.ResponseHeaderTimeout) * time.Second

	// 提取代理 URL
	proxyAddr := ""
	if len(proxyURL) > 0 {
		proxyAddr = proxyURL[0]
	}

	key := fmt.Sprintf("standard-%d-%t-%d-%s", timeout, insecure, envConfig.ResponseHeaderTimeout, proxyAddr)

	if useCache {
		cm.mu.RLock()
		if client, ok := cm.clients[key]; ok {
			cm.mu.RUnlock()
			return client
		}
		cm.mu.RUnlock()
	}

	if useCache {
		cm.mu.Lock()
		defer cm.mu.Unlock()

		if client, ok := cm.clients[key]; ok {
			return client
		}
	}

	transport := &http.Transport{
		MaxIdleConns:          100,
		MaxIdleConnsPerHost:   10,
		IdleConnTimeout:       90 * time.Second,
		DisableCompression:    false,
		TLSHandshakeTimeout:   10 * time.Second,
		ResponseHeaderTimeout: responseHeaderTimeout,
		ExpectContinueTimeout: 1 * time.Second,
		ForceAttemptHTTP2:     true,
	}

	if insecure {
		transport.TLSClientConfig = &tls.Config{InsecureSkipVerify: true}
		envCfg := config.NewEnvConfig()
		if envCfg.IsProduction() {
			log.Printf("[HttpClient-Warn] 生产环境启用了 insecureSkipVerify，存在中间人攻击风险")
		}
	}

	applyProxy(transport, proxyAddr)

	client := &http.Client{
		Transport: transport,
		Timeout:   timeout,
	}

	if useCache {
		cm.clients[key] = client
	}
	return client
}

// GetStreamClient 获取流式客户端（无超时，用于 SSE 流式响应）
// proxyURL: 可选的代理地址（支持 http/https/socks5 协议）
func (cm *ClientManager) GetStreamClient(insecure bool, proxyURL ...string) *http.Client {
	// 从配置获取响应头超时时间
	envConfig := config.NewEnvConfig()
	responseHeaderTimeout := time.Duration(envConfig.ResponseHeaderTimeout) * time.Second

	// 提取代理 URL
	proxyAddr := ""
	if len(proxyURL) > 0 {
		proxyAddr = proxyURL[0]
	}

	key := fmt.Sprintf("stream-%t-%d-%s", insecure, envConfig.ResponseHeaderTimeout, proxyAddr)

	cm.mu.RLock()
	if client, ok := cm.clients[key]; ok {
		cm.mu.RUnlock()
		return client
	}
	cm.mu.RUnlock()

	cm.mu.Lock()
	defer cm.mu.Unlock()

	// 双重检查
	if client, ok := cm.clients[key]; ok {
		return client
	}

	transport := &http.Transport{
		MaxIdleConns:          200, // 流式连接池更大
		MaxIdleConnsPerHost:   20,
		IdleConnTimeout:       120 * time.Second,
		DisableCompression:    true, // 流式响应禁用压缩
		TLSHandshakeTimeout:   10 * time.Second,
		ResponseHeaderTimeout: responseHeaderTimeout,
		ExpectContinueTimeout: 1 * time.Second,
		ForceAttemptHTTP2:     true,
	}

	if insecure {
		transport.TLSClientConfig = &tls.Config{InsecureSkipVerify: true}
	}

	applyProxy(transport, proxyAddr)

	client := &http.Client{
		Transport: transport,
		Timeout:   0, // 流式请求无超时
	}

	cm.clients[key] = client
	return client
}

// applyProxy 为 transport 配置代理
// 支持 http://, https://, socks5:// 协议
func applyProxy(transport *http.Transport, proxyAddr string) {
	if proxyAddr == "" {
		return
	}

	u, err := url.Parse(proxyAddr)
	if err != nil {
		// 对代理 URL 进行脱敏处理，避免泄露凭证
		redactedProxyURL := utils.RedactURLCredentials(proxyAddr)
		log.Printf("[HttpClient-Proxy] 警告: 代理地址解析失败: %s, 错误: %v", redactedProxyURL, err)
		return
	}

	scheme := strings.ToLower(u.Scheme)
	switch scheme {
	case "socks5", "socks5h":
		// SOCKS5 代理：通过 golang.org/x/net/proxy 创建 DialContext
		var auth *proxy.Auth
		if u.User != nil {
			password, _ := u.User.Password()
			auth = &proxy.Auth{
				User:     u.User.Username(),
				Password: password,
			}
		}
		dialer, dialErr := proxy.SOCKS5("tcp", u.Host, auth, proxy.Direct)
		if dialErr != nil {
			redactedProxyURL := utils.RedactURLCredentials(proxyAddr)
			log.Printf("[HttpClient-Proxy] 警告: SOCKS5 代理创建失败: %s, 错误: %v", redactedProxyURL, dialErr)
			return
		}
		// proxy.Dialer 实现了 ContextDialer 接口
		if contextDialer, ok := dialer.(proxy.ContextDialer); ok {
			transport.DialContext = contextDialer.DialContext
		} else {
			// 兜底：将 Dial 包装为 DialContext
			transport.DialContext = func(ctx context.Context, network, addr string) (net.Conn, error) {
				return dialer.Dial(network, addr)
			}
		}
		// SOCKS5 代理不支持 HTTP/2 (需要直连 TLS)
		transport.ForceAttemptHTTP2 = false

	case "http", "https":
		// HTTP/HTTPS 代理
		transport.Proxy = http.ProxyURL(u)

	default:
		log.Printf("[HttpClient-Proxy] 警告: 不支持的代理协议: %s (地址: %s)", scheme, utils.RedactURLCredentials(proxyAddr))
	}
}

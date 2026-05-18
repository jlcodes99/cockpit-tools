package utils

import (
	"fmt"
	"net"
	"net/url"
	"regexp"
	"strings"
)

// reURLPassword 匹配 URL 中 user:password@ 的密码部分
var reURLPassword = regexp.MustCompile(`(://[^:@/]+:)[^@]+(@)`)
var reBaseURLVersionSuffix = regexp.MustCompile(`/v\d+[a-z]*$`)

// RedactURLCredentials 对 URL 中的用户名和密码进行脱敏处理
// 例如: http://user:pass@host:port -> http://user:***@host:port
// 若 URL 解析失败，使用正则兜底替换，避免凭证泄露
func RedactURLCredentials(rawURL string) string {
	if rawURL == "" {
		return rawURL
	}

	u, err := url.Parse(rawURL)
	if err != nil {
		// 解析失败时用正则兜底，避免凭证明文出现在日志中
		return reURLPassword.ReplaceAllString(rawURL, "${1}***${2}")
	}

	if u.User != nil {
		username := u.User.Username()
		// 构建脱敏后的 Userinfo
		u.User = url.UserPassword(username, "***")
		return u.String()
	}

	return rawURL
}

// ValidateBaseURL 验证 baseURL 是否安全，防止 SSRF 攻击
// 仅拦截云元数据服务（169.254.169.254），允许其他内网地址（支持 Ollama、内网部署）
func ValidateBaseURL(rawURL string) error {
	if rawURL == "" {
		return fmt.Errorf("baseURL 不能为空")
	}

	u, err := url.Parse(rawURL)
	if err != nil {
		return fmt.Errorf("无效的 URL 格式: %w", err)
	}

	// 检查协议
	if u.Scheme != "http" && u.Scheme != "https" {
		return fmt.Errorf("不支持的协议: %s（仅允许 http/https）", u.Scheme)
	}

	// 提取主机名（去除端口）
	host := u.Hostname()
	if host == "" {
		return fmt.Errorf("URL 缺少主机名")
	}

	// 硬编码拦截云元数据服务（最关键的安全风险）
	if host == "169.254.169.254" {
		return fmt.Errorf("禁止访问云元数据服务")
	}

	// 检查域名是否解析到云元数据服务
	ips, err := net.LookupIP(host)
	if err != nil {
		// DNS 解析失败，允许通过（避免误杀）
		return nil
	}

	for _, resolvedIP := range ips {
		if resolvedIP.String() == "169.254.169.254" {
			return fmt.Errorf("域名 %s 解析到云元数据服务", host)
		}
	}

	return nil
}

// DefaultVersionPrefixForService 返回服务类型默认自动补齐的版本前缀。
func DefaultVersionPrefixForService(serviceType string) string {
	if strings.EqualFold(serviceType, "gemini") {
		return "/v1beta"
	}
	return "/v1"
}

func normalizeBaseURL(rawURL string) (normalized string, hasHash bool) {
	trimmed := strings.TrimSpace(rawURL)
	if trimmed == "" {
		return "", false
	}

	hasHash = strings.HasSuffix(trimmed, "#")
	withoutHash := strings.TrimSuffix(trimmed, "#")
	normalized = strings.TrimRight(withoutHash, "/")
	return normalized, hasHash
}

// CanonicalBaseURL 返回用户可见的最短等效 BaseURL。
// 规则：忽略尾部 /；保留 # 语义；仅在无 # 时折叠默认自动版本前缀。
func CanonicalBaseURL(rawURL, serviceType string) string {
	normalized, hasHash := normalizeBaseURL(rawURL)
	if normalized == "" {
		return ""
	}
	if hasHash {
		return normalized + "#"
	}

	versionPrefix := DefaultVersionPrefixForService(serviceType)
	if strings.HasSuffix(normalized, versionPrefix) {
		return strings.TrimSuffix(normalized, versionPrefix)
	}
	return normalized
}

// MetricsIdentityBaseURL 返回用于指标归并的稳定 BaseURL 标识。
// 规则：
// 1. 保留 # 语义（避免与普通 URL 合并）
// 2. 已显式带版本后缀时直接使用
// 3. 未带版本后缀时补齐该 serviceType 的默认版本前缀
func MetricsIdentityBaseURL(rawURL, serviceType string) string {
	normalized, hasHash := normalizeBaseURL(rawURL)
	if normalized == "" {
		return ""
	}
	if hasHash {
		return normalized + "#"
	}
	if reBaseURLVersionSuffix.MatchString(normalized) {
		return normalized
	}
	return normalized + DefaultVersionPrefixForService(serviceType)
}

// EquivalentBaseURLVariants 返回与当前 BaseURL 等效的兼容变体，
// 用于兼容旧历史数据中的原始 baseURL / baseURL/ / baseURL+默认版本形式。
func EquivalentBaseURLVariants(rawURL, serviceType string) []string {
	normalized, hasHash := normalizeBaseURL(rawURL)
	if normalized == "" {
		return nil
	}

	variants := make([]string, 0, 4)
	seen := make(map[string]struct{}, 4)
	add := func(value string) {
		if value == "" {
			return
		}
		if _, ok := seen[value]; ok {
			return
		}
		seen[value] = struct{}{}
		variants = append(variants, value)
	}

	if hasHash {
		add(normalized + "#")
		add(normalized + "/#")
		return variants
	}

	versionPrefix := DefaultVersionPrefixForService(serviceType)
	if reBaseURLVersionSuffix.MatchString(normalized) && !strings.HasSuffix(normalized, versionPrefix) {
		add(normalized)
		add(normalized + "/")
		return variants
	}

	canonical := CanonicalBaseURL(rawURL, serviceType)
	identity := MetricsIdentityBaseURL(rawURL, serviceType)
	add(canonical)
	add(canonical + "/")
	add(identity)
	add(identity + "/")
	return variants
}

// isPrivateIP 判断 IP 是否为私有地址（保留用于其他场景）
func isPrivateIP(ip net.IP) bool {
	// IPv4 私有地址段
	privateIPv4Blocks := []string{
		"10.0.0.0/8",     // Class A 私有网络
		"172.16.0.0/12",  // Class B 私有网络
		"192.168.0.0/16", // Class C 私有网络
		"127.0.0.0/8",    // Loopback
		"169.254.0.0/16", // Link-local
		"0.0.0.0/8",      // 当前网络
		"224.0.0.0/4",    // 组播
		"240.0.0.0/4",    // 保留
	}

	// IPv6 私有地址段
	privateIPv6Blocks := []string{
		"::1/128",   // Loopback
		"fc00::/7",  // Unique local
		"fe80::/10", // Link-local
		"ff00::/8",  // 组播
	}

	blocks := privateIPv4Blocks
	if ip.To4() == nil {
		blocks = append(blocks, privateIPv6Blocks...)
	}

	for _, block := range blocks {
		_, subnet, err := net.ParseCIDR(block)
		if err != nil {
			continue
		}
		if subnet.Contains(ip) {
			return true
		}
	}

	// 检查 localhost 域名
	if strings.EqualFold(ip.String(), "localhost") {
		return true
	}

	return false
}

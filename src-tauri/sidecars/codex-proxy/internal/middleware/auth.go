package middleware

import (
	"log"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
)

// isAPIProxyPath 判断是否为 API 代理路径，包括带路由前缀的情况。
// 例如 /v1/responses, /deepseek/v1/responses, /v1beta/models, /bigmodel/v1beta/models 等。
func isAPIProxyPath(path string) bool {
	if strings.HasPrefix(path, "/v1/") || strings.HasPrefix(path, "/v1beta/") {
		return true
	}
	// 检查 /<routePrefix>/v1/ 或 /<routePrefix>/v1beta/ 格式
	parts := strings.SplitN(path, "/", 4)
	if len(parts) >= 3 && (parts[2] == "v1" || parts[2] == "v1beta") {
		return true
	}
	return false
}

// WebAuthMiddleware Web 访问控制中间件
func WebAuthMiddleware(envCfg *config.EnvConfig, cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		path := c.Request.URL.Path

		// 公开端点直接放行（健康检查固定为 /health）
		if path == "/health" {
			c.Next()
			return
		}

		// 静态资源文件直接放行
		if isStaticResource(path) {
			c.Next()
			return
		}

		// API 代理端点后续处理（含路由前缀，如 /deepseek/v1/responses）
		if isAPIProxyPath(path) {
			c.Next()
			return
		}

		// 检查访问密钥（管理 API + 管理端点）
		if strings.HasPrefix(path, "/api") || strings.HasPrefix(path, "/admin") {
			providedKey := getAPIKey(c)
			expectedKey := envCfg.GetAdminAccessKey() // 使用独立的管理密钥

			// 记录认证尝试
			clientIP := c.ClientIP()
			timestamp := time.Now().Format(time.RFC3339)

			if providedKey == "" || providedKey != expectedKey {
				// 认证失败 - 记录详细日志
				reason := "密钥无效"
				if providedKey == "" {
					reason = "密钥缺失"
				}
				log.Printf("[Auth-Failed] IP: %s | Path: %s | Time: %s | Reason: %s",
					clientIP, path, timestamp, reason)

				c.JSON(401, gin.H{
					"error":   "Unauthorized",
					"message": "Invalid or missing access key",
				})
				c.Abort()
				return
			}

			// 认证成功 - 记录日志(可选，根据日志级别)
			// 如果启用了 QuietPollingLogs，则静默轮询端点日志
			if envCfg.ShouldLog("info") && !(envCfg.QuietPollingLogs && isPollingEndpoint(path)) {
				log.Printf("[Auth-Success] IP: %s | Path: %s | Time: %s", clientIP, path, timestamp)
			}
			c.Next()
			return
		}

		// 如果禁用了 Web UI，只禁用网页入口，管理 API 和代理 API 仍然可用。
		if !envCfg.EnableWebUI {
			c.JSON(404, gin.H{
				"error":   "Web界面已禁用",
				"message": "此服务器运行在纯API模式下，请通过API端点访问服务",
			})
			c.Abort()
			return
		}

		// SPA 页面路由直接交给前端处理，但需要排除 /api* 路径
		if path == "/" || path == "/index.html" || (!strings.Contains(path, ".") && !strings.HasPrefix(path, "/api") && !strings.HasPrefix(path, "/admin")) {
			c.Next()
			return
		}

		c.Next()
	}
}

// isPollingEndpoint 判断是否为轮询端点（前缀匹配，兼容 query string 和尾部斜杠）
// 复用 defaultSkipPrefixes 保持与 FilteredLogger 一致
func isPollingEndpoint(path string) bool {
	// 移除 query string
	if idx := strings.Index(path, "?"); idx != -1 {
		path = path[:idx]
	}
	// 移除尾部斜杠
	path = strings.TrimSuffix(path, "/")

	// 复用 logger.go 中的 defaultSkipPrefixes
	for _, prefix := range defaultSkipPrefixes {
		if strings.HasPrefix(path, prefix) {
			return true
		}
	}
	return false
}

// isStaticResource 判断是否为静态资源
func isStaticResource(path string) bool {
	staticExtensions := []string{
		"/assets/", ".css", ".js", ".ico", ".png", ".jpg",
		".gif", ".svg", ".woff", ".woff2", ".ttf", ".eot",
	}

	for _, ext := range staticExtensions {
		if strings.HasPrefix(path, ext) || strings.HasSuffix(path, ext) {
			return true
		}
	}

	return false
}

// getAPIKey 获取 API 密钥
func getAPIKey(c *gin.Context) string {
	// 从 header 获取
	if key := c.GetHeader("x-api-key"); key != "" {
		return key
	}

	if auth := c.GetHeader("Authorization"); auth != "" {
		// 移除 Bearer 前缀
		return strings.TrimPrefix(auth, "Bearer ")
	}

	// 支持 Gemini SDK 的 x-goog-api-key 头部
	if key := c.GetHeader("x-goog-api-key"); key != "" {
		return key
	}

	return ""
}

// ProxyAuthMiddleware 代理访问控制中间件
func ProxyAuthMiddleware(envCfg *config.EnvConfig) gin.HandlerFunc {
	return func(c *gin.Context) {
		providedKey := getAPIKey(c)
		expectedKey := envCfg.ProxyAccessKey

		if providedKey == "" || providedKey != expectedKey {
			if envCfg.ShouldLog("warn") {
				log.Printf("[Auth-Failed] 代理访问密钥验证失败 - IP: %s", c.ClientIP())
			}

			c.JSON(401, gin.H{
				"error": "Invalid proxy access key",
			})
			c.Abort()
			return
		}

		c.Next()
	}
}

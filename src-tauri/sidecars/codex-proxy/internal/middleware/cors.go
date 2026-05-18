package middleware

import (
	"strings"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

// CORSMiddleware CORS 中间件
func CORSMiddleware(envCfg *config.EnvConfig) gin.HandlerFunc {
	return func(c *gin.Context) {
		// 如果未启用 CORS，直接跳过
		if !envCfg.EnableCORS {
			c.Next()
			return
		}

		origin := c.GetHeader("Origin")

		// 开发环境允许所有 localhost 源
		if envCfg.IsDevelopment() {
			if origin != "" && strings.Contains(origin, "localhost") {
				c.Header("Access-Control-Allow-Origin", origin)
			}
		} else {
			// 生产环境使用配置的源
			c.Header("Access-Control-Allow-Origin", envCfg.CORSOrigin)
		}

		c.Header("Access-Control-Allow-Methods", "GET, POST, PUT, PATCH, DELETE, OPTIONS")
		c.Header("Access-Control-Allow-Headers", "Content-Type, Authorization, x-api-key, x-goog-api-key")
		// 仅在非 * 时设置 credentials，避免浏览器拒绝 credentials + * 组合
		if envCfg.CORSOrigin != "*" {
			c.Header("Access-Control-Allow-Credentials", "true")
		}

		// 处理预检请求
		if c.Request.Method == "OPTIONS" {
			c.AbortWithStatus(204)
			return
		}

		c.Next()
	}
}

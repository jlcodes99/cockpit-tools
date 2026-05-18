package middleware

import (
	"strings"

	"github.com/gin-contrib/gzip"
	"github.com/gin-gonic/gin"
)

// GzipMiddleware 为静态资源启用 Gzip 压缩
// 使用白名单模式：仅压缩已知的静态资源路径，避免意外压缩流式响应
func GzipMiddleware() gin.HandlerFunc {
	gzipHandler := gzip.Gzip(gzip.DefaultCompression)

	return func(c *gin.Context) {
		path := c.Request.URL.Path

		// 白名单：仅压缩静态资源
		if isCompressibleResource(path) {
			gzipHandler(c)
			return
		}

		// 其他路径不压缩，直接放行
		c.Next()
	}
}

// isCompressibleResource 判断是否为可压缩的静态资源路径
func isCompressibleResource(path string) bool {
	// 1. Vite 构建产物目录
	if strings.HasPrefix(path, "/assets/") {
		return true
	}

	// 2. 根路径静态文件（HTML、favicon 等）
	if path == "/" || path == "/index.html" || path == "/favicon.ico" {
		return true
	}

	// 3. 按扩展名匹配（兜底）
	staticExts := []string{".js", ".css", ".svg", ".woff", ".woff2", ".ttf", ".eot"}
	for _, ext := range staticExts {
		if strings.HasSuffix(path, ext) {
			return true
		}
	}

	return false
}

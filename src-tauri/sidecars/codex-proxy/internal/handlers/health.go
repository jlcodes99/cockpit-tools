package handlers

import (
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

// HealthCheck 健康检查处理器
func HealthCheck(envCfg *config.EnvConfig, cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		config := cfgManager.GetConfig()

		healthData := gin.H{
			"status":    "healthy",
			"timestamp": time.Now().Format(time.RFC3339),
			"uptime":    time.Since(startTime).Seconds(),
			"mode":      envCfg.Env,
			"version":   getVersion(),
			"config": gin.H{
				"upstreamCount": len(config.Upstream),
			},
		}

		c.JSON(200, healthData)
	}
}

// getVersion 获取版本信息
func getVersion() gin.H {
	// 这些变量在编译时通过 -ldflags 注入
	// 从根目录 VERSION 文件读取
	return gin.H{
		"version":   getVersionString(),
		"buildTime": getBuildTime(),
		"gitCommit": getGitCommit(),
	}
}

// 以下函数用于从 main 包获取版本信息
// 由于无法直接导入 main 包，使用默认值
var (
	versionString = "v0.0.0-dev"
	buildTime     = "unknown"
	gitCommit     = "unknown"
)

func getVersionString() string { return versionString }
func getBuildTime() string     { return buildTime }
func getGitCommit() string     { return gitCommit }

// SetVersionInfo 设置版本信息（从 main 调用）
func SetVersionInfo(version, build, commit string) {
	versionString = version
	buildTime = build
	gitCommit = commit
}

// SaveConfigHandler 配置保存处理器
func SaveConfigHandler(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		if err := cfgManager.SaveConfig(); err != nil {
			c.JSON(500, gin.H{
				"status":    "error",
				"message":   "配置保存失败",
				"error":     err.Error(),
				"timestamp": time.Now().Format(time.RFC3339),
			})
			return
		}

		config := cfgManager.GetConfig()
		c.JSON(200, gin.H{
			"status":    "success",
			"message":   "配置已保存",
			"timestamp": time.Now().Format(time.RFC3339),
			"config": gin.H{
				"upstreamCount": len(config.Upstream),
			},
		})
	}
}

// DevInfo 开发信息处理器
func DevInfo(envCfg *config.EnvConfig, cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		c.JSON(200, gin.H{
			"status":      "development",
			"timestamp":   time.Now().Format(time.RFC3339),
			"config":      cfgManager.GetConfig(),
			"environment": envCfg,
		})
	}
}

var startTime = time.Now()

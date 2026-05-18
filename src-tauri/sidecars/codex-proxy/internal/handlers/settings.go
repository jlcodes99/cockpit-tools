// Package handlers 提供 HTTP 处理器
package handlers

import (
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

// GetFuzzyMode 获取 Fuzzy 模式状态
func GetFuzzyMode(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		c.JSON(200, gin.H{
			"fuzzyModeEnabled": cfgManager.GetFuzzyModeEnabled(),
		})
	}
}

// SetFuzzyMode 设置 Fuzzy 模式状态
func SetFuzzyMode(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		var req struct {
			Enabled bool `json:"enabled"`
		}
		if err := c.ShouldBindJSON(&req); err != nil {
			c.JSON(400, gin.H{"error": "Invalid request body"})
			return
		}

		if err := cfgManager.SetFuzzyModeEnabled(req.Enabled); err != nil {
			c.JSON(500, gin.H{"error": "Failed to save config"})
			return
		}

		c.JSON(200, gin.H{
			"success":          true,
			"fuzzyModeEnabled": req.Enabled,
		})
	}
}

// GetStripBillingHeader 获取移除计费头状态
func GetStripBillingHeader(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		c.JSON(200, gin.H{
			"stripBillingHeader": cfgManager.GetStripBillingHeader(),
		})
	}
}

// SetStripBillingHeader 设置移除计费头状态
func SetStripBillingHeader(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		var req struct {
			Enabled bool `json:"enabled"`
		}
		if err := c.ShouldBindJSON(&req); err != nil {
			c.JSON(400, gin.H{"error": "Invalid request body"})
			return
		}

		if err := cfgManager.SetStripBillingHeader(req.Enabled); err != nil {
			c.JSON(500, gin.H{"error": "Failed to save config"})
			return
		}

		c.JSON(200, gin.H{
			"success":            true,
			"stripBillingHeader": req.Enabled,
		})
	}
}

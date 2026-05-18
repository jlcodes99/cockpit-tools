package handlers

import (
	"strconv"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

// RestoreBlacklistedKey 恢复被拉黑的 API Key
// POST /api/{type}/channels/:id/keys/restore
// Body: {"apiKey": "sk-xxx"}
func RestoreBlacklistedKey(cfgManager *config.ConfigManager, apiType string) gin.HandlerFunc {
	return func(c *gin.Context) {
		idStr := c.Param("id")
		id, err := strconv.Atoi(idStr)
		if err != nil {
			c.JSON(400, gin.H{"error": "Invalid channel ID"})
			return
		}

		var req struct {
			APIKey string `json:"apiKey"`
		}
		if err := c.ShouldBindJSON(&req); err != nil || req.APIKey == "" {
			c.JSON(400, gin.H{"error": "apiKey is required"})
			return
		}

		if err := cfgManager.RestoreKey(apiType, id, req.APIKey); err != nil {
			c.JSON(400, gin.H{"error": err.Error()})
			return
		}

		c.JSON(200, gin.H{
			"message": "Key 已恢复",
			"success": true,
		})
	}
}

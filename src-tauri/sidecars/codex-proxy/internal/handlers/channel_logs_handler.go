package handlers

import (
	"strconv"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
	"github.com/gin-gonic/gin"
)

// GetChannelLogs 获取渠道请求日志
func GetChannelLogs(channelLogStore *metrics.ChannelLogStore) gin.HandlerFunc {
	return func(c *gin.Context) {
		idStr := c.Param("id")
		channelIndex, err := strconv.Atoi(idStr)
		if err != nil {
			c.JSON(400, gin.H{"error": "Invalid channel ID"})
			return
		}

		logs := channelLogStore.Get(channelIndex)
		if logs == nil {
			logs = make([]*metrics.ChannelLog, 0)
		}

		c.JSON(200, gin.H{
			"channelIndex": channelIndex,
			"logs":         logs,
		})
	}
}

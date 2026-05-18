package handlers

import (
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
	"github.com/gin-gonic/gin"
)

// GetModelStatsHistory 获取按模型分组的历史统计
func GetModelStatsHistory(metricsManager *metrics.MetricsManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		durationStr := c.DefaultQuery("duration", "24h")

		var duration time.Duration
		var err error

		if durationStr == "today" {
			duration = metrics.CalculateTodayDuration()
			if duration < time.Minute {
				duration = time.Minute
			}
		} else {
			duration, err = time.ParseDuration(durationStr)
			if err != nil {
				c.JSON(400, gin.H{"error": "Invalid duration parameter. Use: 1h, 6h, 24h, or today"})
				return
			}
		}

		if duration > 24*time.Hour {
			duration = 24 * time.Hour
		}

		// 根据 duration 自动选择聚合粒度
		var interval time.Duration
		switch {
		case duration <= time.Hour:
			interval = time.Minute
		case duration <= 6*time.Hour:
			interval = 5 * time.Minute
		default:
			interval = 15 * time.Minute
		}

		models := metricsManager.GetModelStatsHistory(duration, interval)

		c.JSON(200, gin.H{
			"models":   models,
			"duration": durationStr,
			"interval": interval.String(),
		})
	}
}

package common

import (
	"fmt"
	"log"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/scheduler"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/gin-gonic/gin"
)

// MultiChannelAttemptResult 描述一次"选中渠道"的尝试结果（用于多渠道 failover 外壳复用）。
type MultiChannelAttemptResult struct {
	Handled           bool
	Attempted         bool
	SuccessKey        string
	SuccessBaseURLIdx int
	FailoverError     *FailoverError
	Usage             *types.Usage
	LastError         error
}

// TrySelectedChannelFunc 尝试一次选中的渠道，返回该渠道的尝试结果。
type TrySelectedChannelFunc func(selection *scheduler.SelectionResult) MultiChannelAttemptResult

// OnMultiChannelHandledFunc 在请求被"处理完成"时回调（成功或非 failover 错误都会触发）。
type OnMultiChannelHandledFunc func(selection *scheduler.SelectionResult, result MultiChannelAttemptResult)

// HandleAllFailedFunc 处理"所有渠道都失败"的返回逻辑（不同入口可能有不同错误格式）。
type HandleAllFailedFunc func(c *gin.Context, failoverErr *FailoverError, lastError error)

// HandleMultiChannelFailover 处理多渠道 failover 外壳逻辑（选渠道 + 聚合错误 + Trace 亲和）。
// 具体"渠道内 Key/BaseURL 轮转"由 trySelectedChannel 实现（通常调用 TryUpstreamWithAllKeys）。
func HandleMultiChannelFailover(
	c *gin.Context,
	envCfg *config.EnvConfig,
	channelScheduler *scheduler.ChannelScheduler,
	kind scheduler.ChannelKind,
	apiType string,
	userID string,
	model string,
	trySelectedChannel TrySelectedChannelFunc,
	onHandled OnMultiChannelHandledFunc,
	handleAllFailed HandleAllFailedFunc,
) {
	if c == nil || envCfg == nil || channelScheduler == nil || trySelectedChannel == nil {
		return
	}
	if handleAllFailed == nil {
		handleAllFailed = func(c *gin.Context, failoverErr *FailoverError, lastError error) {
			HandleAllChannelsFailed(c, false, failoverErr, lastError, apiType)
		}
	}

	failedChannels := make(map[int]bool)
	var lastError error
	var lastFailoverError *FailoverError

	maxChannelAttempts := channelScheduler.GetActiveChannelCount(kind)

	for channelAttempt := 0; channelAttempt < maxChannelAttempts; channelAttempt++ {
		// 检查客户端是否已断开连接
		select {
		case <-c.Request.Context().Done():
			if envCfg.ShouldLog("info") {
				log.Printf("[%s-Cancel] 请求已取消，停止渠道 failover", apiType)
			}
			return
		default:
			// 继续正常流程
		}

		selection, err := channelScheduler.SelectChannel(c.Request.Context(), userID, failedChannels, kind, model, c.Param("routePrefix"), c.GetHeader("X-Channel"))
		if err != nil {
			lastError = err
			break
		}

		upstream := selection.Upstream
		channelIndex := selection.ChannelIndex

		if envCfg.ShouldLog("info") && upstream != nil {
			log.Printf("[%s-Select] 选择渠道: [%d] %s (原因: %s, 尝试 %d/%d)",
				apiType, channelIndex, upstream.Name, selection.Reason, channelAttempt+1, maxChannelAttempts)
		}

		result := trySelectedChannel(selection)
		if result.Handled {
			if onHandled != nil {
				onHandled(selection, result)
			}
			// 只有真正成功的请求才设置 Trace 亲和（客户端取消时 SuccessKey 为空）
			if result.SuccessKey != "" {
				channelScheduler.SetTraceAffinity(userID, channelIndex, kind)
			}
			return
		}

		failedChannels[channelIndex] = true

		if result.FailoverError != nil {
			lastFailoverError = result.FailoverError
			if upstream != nil {
				lastError = fmt.Errorf("渠道 [%d] %s 失败", channelIndex, upstream.Name)
			} else {
				lastError = fmt.Errorf("渠道 [%d] 失败", channelIndex)
			}
		}

		if result.Attempted && upstream != nil {
			log.Printf("[%s-Failover] 警告: 渠道 [%d] %s 所有密钥都失败，尝试下一个渠道", apiType, channelIndex, upstream.Name)
		}
	}

	log.Printf("[%s-Error] 所有渠道都失败了", apiType)
	handleAllFailed(c, lastFailoverError, lastError)
}

package handlers

import (
	"context"
	"fmt"
	"log"
	"net/http"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
	"github.com/gin-gonic/gin"
)

// ============== 取消与重测 ==============

// CancelCapabilityTestJob 取消正在进行的能力测试
func CancelCapabilityTestJob(cfgManager *config.ConfigManager, channelKind string) gin.HandlerFunc {
	return func(c *gin.Context) {
		id, err := parseCapabilityChannelID(c)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid channel ID"})
			return
		}

		jobID := c.Param("jobId")
		job, ok := capabilityJobs.get(jobID)
		if !ok {
			c.JSON(http.StatusNotFound, gin.H{"error": "Capability test job not found"})
			return
		}

		channel, chErr := getCapabilityTestChannel(cfgManager, channelKind, id)
		if chErr != nil {
			if !capabilityJobMatchesChannel(job, nil, channelKind, id) {
				c.JSON(http.StatusNotFound, gin.H{"error": "Capability test job not found"})
				return
			}
		} else if !capabilityJobMatchesChannel(job, channel, channelKind, id) {
			c.JSON(http.StatusNotFound, gin.H{"error": "Capability test job not found"})
			return
		}

		// 只能取消正在运行或排队中的 job
		if job.Status != CapabilityJobStatusRunning && job.Status != CapabilityJobStatusQueued {
			c.JSON(http.StatusConflict, gin.H{"error": "Job is not running"})
			return
		}

		// 调用 CancelFunc 取消 goroutine
		if cancelFn, ok := capabilityJobs.getCancelFunc(jobID); ok {
			cancelFn()
		}

		// 更新 job 状态
		capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
			job.Status = CapabilityJobStatusCancelled
			job.Lifecycle = CapabilityLifecycleCancelled
			job.Outcome = CapabilityOutcomeCancelled
			job.FinishedAt = time.Now().Format(time.RFC3339Nano)
			for i := range job.Tests {
				for j := range job.Tests[i].ModelResults {
					switch job.Tests[i].ModelResults[j].Status {
					case CapabilityModelStatusQueued:
						job.Tests[i].ModelResults[j].Status = CapabilityModelStatusSkipped
						job.Tests[i].ModelResults[j].Lifecycle = CapabilityLifecycleDone
						job.Tests[i].ModelResults[j].Outcome = CapabilityOutcomeUnknown
						reason := "not_run"
						job.Tests[i].ModelResults[j].Reason = &reason
					case CapabilityModelStatusRunning:
						job.Tests[i].ModelResults[j].Status = CapabilityModelStatusSkipped
						job.Tests[i].ModelResults[j].Lifecycle = CapabilityLifecycleCancelled
						job.Tests[i].ModelResults[j].Outcome = CapabilityOutcomeCancelled
						reason := "cancelled"
						job.Tests[i].ModelResults[j].Reason = &reason
						job.Tests[i].ModelResults[j].Error = &reason
					}
				}
				job.Tests[i].Lifecycle = CapabilityLifecycleCancelled
				job.Tests[i].Outcome = CapabilityOutcomeCancelled
				reason := "cancelled"
				job.Tests[i].Reason = &reason
			}
		})

		log.Printf("[CapabilityTest-Cancel] 能力测试任务 %s 已取消", jobID)
		c.JSON(http.StatusOK, gin.H{"status": "cancelled"})
	}
}

// RetryCapabilityTestModel 重测单个模型
func RetryCapabilityTestModel(cfgManager *config.ConfigManager, channelLogStore *metrics.ChannelLogStore, channelKind string) gin.HandlerFunc {
	return func(c *gin.Context) {
		id, err := parseCapabilityChannelID(c)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid channel ID"})
			return
		}

		jobID := c.Param("jobId")
		job, ok := capabilityJobs.get(jobID)
		if !ok {
			c.JSON(http.StatusNotFound, gin.H{"error": "Capability test job not found"})
			return
		}

		channel, chErr := getCapabilityTestChannel(cfgManager, channelKind, id)
		if chErr != nil {
			if !capabilityJobMatchesChannel(job, nil, channelKind, id) {
				c.JSON(http.StatusNotFound, gin.H{"error": "Capability test job not found"})
				return
			}
		} else if !capabilityJobMatchesChannel(job, channel, channelKind, id) {
			c.JSON(http.StatusNotFound, gin.H{"error": "Capability test job not found"})
			return
		}

		var req struct {
			Protocol string `json:"protocol"`
			Model    string `json:"model"`
		}
		if err := c.ShouldBindJSON(&req); err != nil || req.Protocol == "" || req.Model == "" {
			c.JSON(http.StatusBadRequest, gin.H{"error": "protocol and model are required"})
			return
		}

		// 仅允许在终态 job 上执行单模型重测，避免与主任务并发冲突导致状态抖动
		if job.Lifecycle == CapabilityLifecyclePending || job.Lifecycle == CapabilityLifecycleActive ||
			job.Status == CapabilityJobStatusQueued || job.Status == CapabilityJobStatusRunning {
			c.JSON(http.StatusConflict, gin.H{"error": "Capability test job is still running"})
			return
		}

		// 检查模型是否存在于 job 中；若协议存在但模型不在（如快照新增的探测模型），
		// 自动为该协议追加一个 idle 状态的模型结果，使其可被重测
		modelFound := false
		modelRetryable := false
		protocolFound := false
		for _, test := range job.Tests {
			if test.Protocol != req.Protocol {
				continue
			}
			protocolFound = true
			for _, mr := range test.ModelResults {
				if mr.Model == req.Model {
					modelFound = true
					// 允许重测：失败、跳过、取消、或未测试（idle）
					if mr.Status == CapabilityModelStatusFailed ||
						mr.Status == CapabilityModelStatusSkipped ||
						mr.Outcome == CapabilityOutcomeCancelled ||
						mr.Lifecycle == CapabilityLifecycleCancelled ||
						mr.Status == CapabilityModelStatusIdle {
						modelRetryable = true
					}
					break
				}
			}
		}
		if !modelFound && protocolFound && strings.Contains(req.Protocol, "->") {
			// 协议存在但模型不在，追加 idle 占位以允许测试
			capabilityJobs.update(jobID, func(j *CapabilityTestJob) {
				for i := range j.Tests {
					if j.Tests[i].Protocol != req.Protocol {
						continue
					}
					j.Tests[i].ModelResults = append(j.Tests[i].ModelResults, CapabilityModelJobResult{
						Model:  req.Model,
						Status: CapabilityModelStatusIdle,
					})
					break
				}
			})
			modelFound = true
			modelRetryable = true
		}
		if !modelFound {
			c.JSON(http.StatusNotFound, gin.H{"error": "Model not found in job"})
			return
		}
		if !modelRetryable {
			c.JSON(http.StatusConflict, gin.H{"error": "Model is not retryable"})
			return
		}

		// 获取渠道配置
		channel, chErr = getCapabilityTestChannel(cfgManager, channelKind, id)
		if chErr != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": chErr.Error()})
			return
		}

		timeout := 30 * time.Second
		if job.TimeoutMilliseconds > 0 {
			timeout = time.Duration(job.TimeoutMilliseconds) * time.Millisecond
		}

		// 将 job/协议/模型切换到单模型重测态
		retryStartedAt := time.Now().Format(time.RFC3339Nano)
		capabilityJobs.update(jobID, func(j *CapabilityTestJob) {
			for i := range j.Tests {
				if j.Tests[i].Protocol != req.Protocol {
					continue
				}
				j.Tests[i].Lifecycle = CapabilityLifecycleActive
				j.Tests[i].Outcome = CapabilityOutcomeUnknown
				j.Tests[i].Status = CapabilityProtocolStatusRunning
				j.Tests[i].Reason = nil
				j.Tests[i].Error = nil
				updateCapabilityJobModelResult(j, req.Protocol, req.Model, CapabilityModelStatusRunning, ModelTestResult{
					Model:     req.Model,
					StartedAt: retryStartedAt,
				})
				break
			}
			j.Lifecycle = CapabilityLifecycleActive
			j.Outcome = CapabilityOutcomeUnknown
			j.Status = CapabilityJobStatusRunning
			j.FinishedAt = ""
		})

		// 异步执行单模型测试（使用独立可取消 context）
		// 不覆盖 job 的 CancelFunc，避免影响主任务的取消能力
		retryCtx, retryCancel := context.WithCancel(context.Background())

		go func() {
			defer retryCancel()
			apiKey := ""
			if len(channel.APIKeys) > 0 {
				apiKey = channel.APIKeys[0]
			} else if len(channel.DisabledAPIKeys) > 0 {
				apiKey = channel.DisabledAPIKeys[0].Key
			}

			baseURL := ""
			if len(channel.GetAllBaseURLs()) > 0 {
				baseURL = channel.GetAllBaseURLs()[0]
			}
			identityKey := metrics.GenerateMetricsIdentityKey(baseURL, apiKey, channel.ServiceType)
			retryRPM := job.EffectiveRPM
			if retryRPM <= 0 {
				retryRPM = 10
			}
			if retryRPM > 60 {
				retryRPM = 60
			}
			interval := time.Minute / time.Duration(retryRPM)

			if err := GetCapabilityTestDispatcher(identityKey).AcquirePrioritySendSlot(retryCtx, interval); err != nil {
				log.Printf("[CapabilityTest-Retry] 获取优先发送槽位失败: job=%s, protocol=%s, model=%s, err=%v",
					jobID, req.Protocol, req.Model, err)
				capabilityJobs.update(jobID, func(j *CapabilityTestJob) {
					errMsg := fmt.Sprintf("retry_queue_cancelled: %v", err)
					updateCapabilityJobModelResult(j, req.Protocol, req.Model, CapabilityModelStatusFailed, ModelTestResult{
						Model:    req.Model,
						Error:    &errMsg,
						TestedAt: time.Now().Format(time.RFC3339Nano),
					})
				})
				return
			}

			modelResult := executeRetryModelTest(retryCtx, channel, req.Protocol, req.Model, timeout, jobID, cfgManager, id, channelKind, apiKey, channelLogStore)

			// 更新协议测试时间戳；协议/任务整体状态由统一重算逻辑维护
			capabilityJobs.update(jobID, func(j *CapabilityTestJob) {
				for i := range j.Tests {
					if j.Tests[i].Protocol != req.Protocol {
						continue
					}
					j.Tests[i].TestedAt = time.Now().Format(time.RFC3339Nano)
					break
				}
			})

			log.Printf("[CapabilityTest-Retry] 单模型重测完成: job=%s, protocol=%s, model=%s, success=%v",
				jobID, req.Protocol, req.Model, modelResult.Success)
		}()

		c.JSON(http.StatusOK, gin.H{"status": "accepted"})
	}
}

func executeRetryModelTest(ctx context.Context, channel *config.UpstreamConfig, protocol, model string, timeout time.Duration, jobID string, cfgManager *config.ConfigManager, channelID int, channelKind, apiKey string, channelLogStore *metrics.ChannelLogStore) ModelTestResult {
	if !strings.Contains(protocol, "->") {
		return executeModelTest(ctx, channel, protocol, model, timeout, jobID, cfgManager, channelID, channelKind, apiKey, channelLogStore)
	}

	parts := strings.SplitN(protocol, "->", 2)
	if len(parts) != 2 || parts[1] == "" {
		errMsg := fmt.Sprintf("invalid_virtual_protocol: %s", protocol)
		result := ModelTestResult{
			Model:    model,
			Error:    &errMsg,
			TestedAt: time.Now().Format(time.RFC3339Nano),
		}
		capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
			updateCapabilityJobModelResult(job, protocol, model, CapabilityModelStatusFailed, result)
		})
		return result
	}

	actualModel := config.RedirectModel(model, channel)
	redirectResult := executeRedirectModelTest(ctx, channel, parts[1], model, actualModel, timeout, jobID, cfgManager, channelID, apiKey, channelLogStore)
	modelStatus := CapabilityModelStatusFailed
	if redirectResult.Success {
		modelStatus = CapabilityModelStatusSuccess
	}
	result := ModelTestResult{
		Model:              model,
		ActualModel:        redirectResult.ActualModel,
		Success:            redirectResult.Success,
		Latency:            redirectResult.Latency,
		StreamingSupported: redirectResult.StreamingSupported,
		Error:              redirectResult.Error,
		StartedAt:          redirectResult.StartedAt,
		TestedAt:           redirectResult.TestedAt,
	}
	capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
		updateCapabilityJobModelResult(job, protocol, model, modelStatus, result)
	})
	return result
}

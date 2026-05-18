package handlers

import (
	"log"
	"net/http"
	"strconv"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
	"github.com/gin-gonic/gin"
)

// ============== 类型定义 ==============

// CapabilityTestRequest 能力测试请求体
type CapabilityTestRequest struct {
	TargetProtocols []string `json:"targetProtocols"`
	Models          []string `json:"models"`        // 可选：用户指定要测试的模型列表，为空时使用预定义列表
	Timeout         int      `json:"timeout"`       // 毫秒
	PreviousJobID   string   `json:"previousJobId"` // 可选：上次测试的 jobId，用于复用成功结果
	RPM             int      `json:"rpm"`
	SourceTab       string   `json:"sourceTab"` // 可选：当前 Tab 的协议类型（用于跨协议测试）
}

type ModelTestResult struct {
	Model              string  `json:"model"`
	ActualModel        string  `json:"actualModel,omitempty"` // 经 ModelMapping 重定向后实际发送给上游的模型名
	Success            bool    `json:"success"`
	Skipped            bool    `json:"skipped,omitempty"`
	Latency            int64   `json:"latency"` // 毫秒
	StreamingSupported bool    `json:"streamingSupported"`
	Error              *string `json:"error,omitempty"`
	StartedAt          string  `json:"startedAt,omitempty"`
	TestedAt           string  `json:"testedAt"`
}

// ProtocolTestResult 单个协议测试结果
type ProtocolTestResult struct {
	Protocol           string            `json:"protocol"`
	Success            bool              `json:"success"`
	Latency            int64             `json:"latency"` // 毫秒
	StreamingSupported bool              `json:"streamingSupported"`
	TestedModel        string            `json:"testedModel"` // 优先返回首个成功模型名称，兼容旧字段
	ModelResults       []ModelTestResult `json:"modelResults,omitempty"`
	SuccessCount       int               `json:"successCount,omitempty"`
	AttemptedModels    int               `json:"attemptedModels,omitempty"`
	Error              *string           `json:"error"`
	TestedAt           string            `json:"testedAt"`
}

// CapabilityTestResponse 能力测试响应体
type CapabilityTestResponse struct {
	ChannelID           int                   `json:"channelId"`
	ChannelName         string                `json:"channelName"`
	SourceType          string                `json:"sourceType"`
	Tests               []ProtocolTestResult  `json:"tests"`
	RedirectTests       []RedirectModelResult `json:"redirectTests,omitempty"`
	CompatibleProtocols []string              `json:"compatibleProtocols"`
	TotalDuration       int64                 `json:"totalDuration"` // 毫秒
}

// ============== 主处理器 ==============

// TestChannelCapability 渠道能力测试处理器
// channelKind 决定从哪个配置获取渠道：messages/responses/gemini/chat
func TestChannelCapability(cfgManager *config.ConfigManager, channelLogStore *metrics.ChannelLogStore, channelKind string) gin.HandlerFunc {
	return func(c *gin.Context) {
		id, err := strconv.Atoi(c.Param("id"))
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid channel ID"})
			return
		}

		channel, err := getCapabilityTestChannel(cfgManager, channelKind, id)
		if err != nil {
			statusCode := http.StatusBadRequest
			if err.Error() == "channel not found" {
				statusCode = http.StatusNotFound
			}
			c.JSON(statusCode, gin.H{"error": err.Error()})
			return
		}

		var req CapabilityTestRequest
		if err := c.ShouldBindJSON(&req); err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid request body"})
			return
		}

		timeout := 30 * time.Second
		if req.Timeout > 0 {
			timeout = time.Duration(req.Timeout) * time.Millisecond
		}

		protocols := req.TargetProtocols
		if len(protocols) == 0 {
			protocols = []string{"messages", "responses", "chat", "gemini"}
		}

		effectiveRPM := req.RPM
		if effectiveRPM <= 0 {
			effectiveRPM = 10
		}
		if effectiveRPM > 60 {
			effectiveRPM = 60
		}

		if len(channel.APIKeys) == 0 && len(channel.DisabledAPIKeys) == 0 {
			errMsg := "no_api_key"
			resp := CapabilityTestResponse{
				ChannelID:           id,
				ChannelName:         channel.Name,
				SourceType:          channel.ServiceType,
				Tests:               []ProtocolTestResult{},
				CompatibleProtocols: []string{},
				TotalDuration:       0,
			}
			job := createCapabilityJobFromResponse(id, channel.Name, channelKind, channel.ServiceType, protocols, timeout, effectiveRPM, resp, false)
			job.Lifecycle = CapabilityLifecycleDone
			job.Outcome = CapabilityOutcomeFailed
			job.Status = deriveCapabilityJobStatus(job.Lifecycle, job.Outcome)
			job.RunMode = CapabilityRunModeFresh
			job.Error = &errMsg
			capabilityJobs.create(job)
			c.JSON(http.StatusOK, gin.H{"jobId": job.JobID, "resumed": false, "job": job})
			return
		}

		baseURL := ""
		if len(channel.GetAllBaseURLs()) > 0 {
			baseURL = channel.GetAllBaseURLs()[0]
		}
		apiKey := ""
		if len(channel.APIKeys) > 0 {
			apiKey = channel.APIKeys[0]
		} else if len(channel.DisabledAPIKeys) > 0 {
			apiKey = channel.DisabledAPIKeys[0].Key
		}

		normalizedModels := normalizeCapabilityModels(req.Models)
		identityKey := metrics.GenerateMetricsIdentityKey(baseURL, apiKey, channel.ServiceType)
		modelMappingHash := hashModelMapping(channel.ModelMapping)
		cacheKey := buildCapabilityCacheKey(baseURL, apiKey, channel.ServiceType, protocols, normalizedModels, modelMappingHash)
		executionLookupKey := buildCapabilityExecutionLookupKey(identityKey, channelKind, protocols, normalizedModels, modelMappingHash)
		lookupKey := buildCapabilityJobLookupKey(cacheKey, channelKind, id)

		if cached, ok := getCapabilityCache(cacheKey); ok {
			log.Printf("[CapabilityTest-Cache] 渠道 %s (ID:%d) 命中缓存，创建已完成任务", channel.Name, id)
			cached.ChannelID = id
			cached.ChannelName = channel.Name
			cached.SourceType = channel.ServiceType
			job := createCapabilityJobFromResponse(id, channel.Name, channelKind, channel.ServiceType, protocols, timeout, effectiveRPM, *cached, true)
			job.RunMode = CapabilityRunModeCacheHit
			job.CacheHit = true
			job.IdentityKey = identityKey
			job.ExecutionKey = ""
			job.SummaryReason = "cache_hit"
			job.IsResumed = false
			capabilityJobs.create(job)
			c.JSON(http.StatusOK, gin.H{"jobId": job.JobID, "resumed": false, "job": job})
			return
		}

		job, reused := capabilityJobs.getOrCreateByLookupKey(executionLookupKey, func() *CapabilityTestJob {
			created := newCapabilityTestJobWithModels(id, channel.Name, channelKind, channel.ServiceType, protocols, timeout, effectiveRPM, normalizedModels)
			created.IdentityKey = identityKey
			created.ExecutionKey = executionLookupKey
			return created
		})
		if reused && job.Lifecycle == CapabilityLifecycleCancelled {
			capabilityJobs.clearLookupKey(executionLookupKey)
			job = newCapabilityTestJobWithModels(id, channel.Name, channelKind, channel.ServiceType, protocols, timeout, effectiveRPM, normalizedModels)
			job.IdentityKey = identityKey
			job.ExecutionKey = executionLookupKey
			capabilityJobs.create(job)
			reused = false
		}
		capabilityJobs.bindLookupKey(lookupKey, job.JobID)
		job.ChannelID = id
		job.ChannelName = channel.Name
		job.SourceType = channel.ServiceType
		job.IsResumed = reused
		job.IdentityKey = identityKey
		job.ExecutionKey = executionLookupKey

		// 检测到 cancelled job，恢复进度
		if reused && job.Lifecycle == CapabilityLifecycleCancelled {
			log.Printf("[CapabilityTest-Job] 恢复已取消的任务 %s，渠道 %s (ID:%d)", job.JobID, channel.Name, id)

			// 提取已成功的模型作为 previousResults
			previousResults := make(map[string]map[string]ModelTestResult)
			for _, test := range job.Tests {
				modelMap := make(map[string]ModelTestResult)
				for _, mr := range test.ModelResults {
					if mr.Outcome == CapabilityOutcomeSuccess {
						modelMap[mr.Model] = ModelTestResult{
							Model:              mr.Model,
							ActualModel:        mr.ActualModel,
							Success:            mr.Success,
							Latency:            mr.Latency,
							StreamingSupported: mr.StreamingSupported,
							Error:              mr.Error,
							StartedAt:          mr.StartedAt,
							TestedAt:           mr.TestedAt,
						}
					}
				}
				if len(modelMap) > 0 {
					previousResults[test.Protocol] = modelMap
				}
			}

			// 重置 failed/skipped 模型为 queued，准备重测
			updatedJob, ok := capabilityJobs.update(job.JobID, func(j *CapabilityTestJob) {
				j.Lifecycle = CapabilityLifecyclePending
				j.Outcome = CapabilityOutcomeUnknown
				j.Status = deriveCapabilityJobStatus(j.Lifecycle, j.Outcome)
				j.RunMode = CapabilityRunModeResumedCancelled
				j.SummaryReason = "resumed_cancelled"
				j.IsResumed = true
				j.HasReusedResults = len(previousResults) > 0
				j.EffectiveRPM = effectiveRPM
				j.FinishedAt = ""
				for i := range j.Tests {
					if j.Tests[i].Lifecycle == CapabilityLifecycleCancelled || j.Tests[i].Outcome == CapabilityOutcomeFailed {
						j.Tests[i].Lifecycle = CapabilityLifecyclePending
						j.Tests[i].Outcome = CapabilityOutcomeUnknown
						j.Tests[i].Reason = nil
					}
					for k := range j.Tests[i].ModelResults {
						if j.Tests[i].ModelResults[k].Outcome == CapabilityOutcomeFailed ||
							j.Tests[i].ModelResults[k].Status == CapabilityModelStatusSkipped {
							j.Tests[i].ModelResults[k].Lifecycle = CapabilityLifecyclePending
							j.Tests[i].ModelResults[k].Outcome = CapabilityOutcomeUnknown
							j.Tests[i].ModelResults[k].Error = nil
							j.Tests[i].ModelResults[k].Reason = nil
						}
					}
				}
			})
			if !ok {
				c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to resume cancelled job"})
				return
			}

			go runCapabilityTestJob(job.JobID, channelKind, id, *channel, protocols, timeout, effectiveRPM, cacheKey, lookupKey, identityKey, previousResults, normalizedModels, req.SourceTab, cfgManager, channelLogStore)

			c.JSON(http.StatusOK, gin.H{"jobId": updatedJob.JobID, "resumed": true, "job": updatedJob})
			return
		}

		// 复用正在运行的 job
		if reused {
			log.Printf("[CapabilityTest-Job] 复用能力测试任务 %s，渠道 %s (ID:%d, 类型:%s)", job.JobID, channel.Name, id, channel.ServiceType)
			job.RunMode = CapabilityRunModeReusedRunning
			job.SummaryReason = "reused_running"
			job.IsResumed = true
			c.JSON(http.StatusOK, gin.H{"jobId": job.JobID, "resumed": true, "job": job})
			return
		}

		// 创建新 job
		log.Printf("[CapabilityTest-Job] 创建能力测试任务 %s，渠道 %s (ID:%d, 类型:%s)，协议: %v", job.JobID, channel.Name, id, channel.ServiceType, protocols)

		// 提取上次成功的结果用于复用（从 previousJobID）
		var previousResults map[string]map[string]ModelTestResult
		if req.PreviousJobID != "" {
			if prevJob, ok := capabilityJobs.get(req.PreviousJobID); ok && prevJob.ChannelKind == channelKind && prevJob.IdentityKey == identityKey {
				previousResults = make(map[string]map[string]ModelTestResult)
				for _, test := range prevJob.Tests {
					modelMap := make(map[string]ModelTestResult)
					for _, mr := range test.ModelResults {
						if mr.Status == CapabilityModelStatusSuccess {
							modelMap[mr.Model] = ModelTestResult{
								Model:              mr.Model,
								ActualModel:        mr.ActualModel,
								Success:            mr.Success,
								Latency:            mr.Latency,
								StreamingSupported: mr.StreamingSupported,
								Error:              mr.Error,
								StartedAt:          mr.StartedAt,
								TestedAt:           mr.TestedAt,
							}
						}
					}
					if len(modelMap) > 0 {
						previousResults[test.Protocol] = modelMap
					}
				}
				if len(previousResults) > 0 {
					job.RunMode = CapabilityRunModeReusedPreviousResult
					job.SummaryReason = "reused_previous_results"
					job.HasReusedResults = true
					log.Printf("[CapabilityTest-Job] 复用上次测试 %s 的成功结果，跳过 %d 个协议的成功模型",
						req.PreviousJobID, len(previousResults))
				}
			}
		}

		go runCapabilityTestJob(job.JobID, channelKind, id, *channel, protocols, timeout, effectiveRPM, cacheKey, lookupKey, identityKey, previousResults, normalizedModels, req.SourceTab, cfgManager, channelLogStore)

		c.JSON(http.StatusOK, gin.H{"jobId": job.JobID, "resumed": false, "job": job})
		return
	}
}

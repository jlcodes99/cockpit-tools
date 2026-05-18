package handlers

import (
	"context"
	"fmt"
	"log"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/handlers/common"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
)

// ============== 重定向验证 ==============

// runRedirectVerification 测试模型重定向：
// 1. 测试当前渠道类型原生探测模型经 ModelMapping 重定向后是否在上游可用
// 2. 如果提供了 sourceTab，测试该协议的所有探测模型在当前渠道上的可用性（跨协议转换）
func runRedirectVerification(ctx context.Context, channel *config.UpstreamConfig, channelKind, sourceTab string, perModelTimeout time.Duration, effectiveRPM int, jobID string, cfgManager *config.ConfigManager, channelID int, apiKey, dispatcherKey string, channelLogStore *metrics.ChannelLogStore, userModels []string) []RedirectModelResult {
	var redirectedModels []RedirectModelResult

	channelServiceType := serviceTypeToChannelKind(channel.ServiceType)

	log.Printf("[RedirectTest-Debug] 渠道 %s (入口类型:%s, 上游类型:%s), sourceTab=%s", channel.Name, channelKind, channelServiceType, sourceTab)

	// 如果没有 sourceTab，不进行重定向测试
	if sourceTab == "" {
		return nil
	}

	// 只测试用户选择的 sourceTab 对应协议的模型重定向（按探测模型顺序，去重 actualModel）
	probeModels, err := getCapabilityProbeModels(sourceTab)
	if err != nil {
		log.Printf("[RedirectTest-Skip] 获取源协议 %s 的探测模型列表失败: %v", sourceTab, err)
		return nil
	}

	probeModels = filterCapabilityProbeModels(probeModels, userModels)

	testedActualModels := make(map[string]bool)
	for _, m := range probeModels {
		actual := config.RedirectModel(m, channel)
		if testedActualModels[actual] {
			continue
		}
		redirectedModels = append(redirectedModels, RedirectModelResult{
			ProbeModel:  m,
			ActualModel: actual,
		})
		testedActualModels[actual] = true
	}

	if len(redirectedModels) == 0 {
		return nil
	}

	log.Printf("[RedirectTest-Start] 渠道 %s (类型:%s) 命中重定向模型: %d 个", channel.Name, channelKind, len(redirectedModels))

	// 初始化虚拟协议占位符，用于实时更新模型状态
	virtualProtocol := sourceTab + "->" + channelServiceType
	capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
		// 构建虚拟协议的 ModelResults（包含所有探测模型，重定向模型标记 actualModel）
		modelResults := make([]CapabilityModelJobResult, 0, len(probeModels))
		for _, probeModel := range probeModels {
			actualModel := config.RedirectModel(probeModel, channel)
			result := CapabilityModelJobResult{
				Model:     probeModel,
				Status:    CapabilityModelStatusQueued,
				Lifecycle: CapabilityLifecyclePending,
				Outcome:   CapabilityOutcomeUnknown,
			}
			if actualModel != probeModel {
				result.ActualModel = actualModel
			}
			modelResults = append(modelResults, result)
		}

		// 检查虚拟协议是否已存在
		foundIdx := -1
		for i := range job.Tests {
			if job.Tests[i].Protocol == virtualProtocol {
				foundIdx = i
				break
			}
		}
		if foundIdx < 0 {
			// 不存在则创建
			job.Tests = append([]CapabilityProtocolJobResult{{
				Protocol:        virtualProtocol,
				Status:          CapabilityProtocolStatusQueued,
				Lifecycle:       CapabilityLifecyclePending,
				Outcome:         CapabilityOutcomeUnknown,
				AttemptedModels: len(modelResults),
				ModelResults:    modelResults,
				TestedAt:        time.Now().Format(time.RFC3339Nano),
			}}, job.Tests...)
		} else {
			if len(job.Tests[foundIdx].ModelResults) == 0 || len(userModels) > 0 {
				job.Tests[foundIdx].ModelResults = modelResults
				job.Tests[foundIdx].AttemptedModels = len(modelResults)
			}
		}
	})

	interval := time.Minute / time.Duration(effectiveRPM)
	if interval <= 0 {
		interval = time.Minute / 10
	}

	results := make([]RedirectModelResult, 0, len(redirectedModels))
	for _, rt := range redirectedModels {
		if ctx.Err() != nil {
			log.Printf("[RedirectTest-Timeout] 全局上下文取消，终止重定向验证")
			break
		}
		if err := GetCapabilityTestDispatcher(dispatcherKey).AcquireSendSlot(ctx, interval); err != nil {
			log.Printf("[RedirectTest-Dispatcher] 获取发送槽位失败: %v", err)
			break
		}
		// 实时更新同一 actualModel 对应的所有探测模型状态为 running
		startedAt := time.Now().Format(time.RFC3339Nano)
		capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
			updateCapabilityJobModelResultsByActualModel(job, virtualProtocol, rt.ActualModel, CapabilityModelStatusRunning, ModelTestResult{
				ActualModel: rt.ActualModel,
				StartedAt:   startedAt,
			})
			// 标记协议为 active
			for i := range job.Tests {
				if job.Tests[i].Protocol == virtualProtocol {
					job.Tests[i].Lifecycle = CapabilityLifecycleActive
					job.Tests[i].Status = CapabilityProtocolStatusRunning
					break
				}
			}
		})
		result := executeRedirectModelTest(ctx, channel, channelServiceType, rt.ProbeModel, rt.ActualModel, perModelTimeout, jobID, cfgManager, channelID, apiKey, channelLogStore)
		results = append(results, result)
		// 实时更新模型状态为 success/failed
		modelStatus := CapabilityModelStatusFailed
		if result.Success {
			modelStatus = CapabilityModelStatusSuccess
		}
		capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
			updateCapabilityJobModelResultsByActualModel(job, virtualProtocol, rt.ActualModel, modelStatus, ModelTestResult{
				ActualModel:        rt.ActualModel,
				Success:            result.Success,
				Latency:            result.Latency,
				StreamingSupported: result.StreamingSupported,
				Error:              result.Error,
				StartedAt:          result.StartedAt,
				TestedAt:           result.TestedAt,
			})
		})
	}

	// 更新虚拟协议的最终状态
	// 构建 actualModel -> 测试结果的映射
	actualModelResults := make(map[string]RedirectModelResult)
	for _, r := range results {
		actualModelResults[r.ActualModel] = r
	}

	capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
		for i := range job.Tests {
			if job.Tests[i].Protocol == virtualProtocol {
				// 更新所有模型的测试结果（共享 actualModel 的模型使用相同结果）
				successCount := 0
				for j := range job.Tests[i].ModelResults {
					actualModel := job.Tests[i].ModelResults[j].ActualModel
					if actualModel == "" {
						actualModel = job.Tests[i].ModelResults[j].Model
					}
					if result, ok := actualModelResults[actualModel]; ok {
						modelStatus := CapabilityModelStatusFailed
						if result.Success {
							modelStatus = CapabilityModelStatusSuccess
							successCount++
						}
						updateCapabilityJobModelResult(job, virtualProtocol, job.Tests[i].ModelResults[j].Model, modelStatus, ModelTestResult{
							Model:              job.Tests[i].ModelResults[j].Model,
							ActualModel:        actualModel,
							Success:            result.Success,
							Latency:            result.Latency,
							StreamingSupported: result.StreamingSupported,
							Error:              result.Error,
							StartedAt:          result.StartedAt,
							TestedAt:           result.TestedAt,
						})
					}
				}

				// 更新协议状态
				job.Tests[i].Lifecycle = CapabilityLifecycleDone
				job.Tests[i].Status = CapabilityProtocolStatusCompleted
				if successCount > 0 {
					job.Tests[i].Outcome = CapabilityOutcomeSuccess
					job.Tests[i].Success = true
					job.Tests[i].SuccessCount = successCount
				} else {
					job.Tests[i].Outcome = CapabilityOutcomeFailed
					job.Tests[i].Success = false
					errMsg := "all_models_failed"
					job.Tests[i].Error = &errMsg
				}
				job.Tests[i].TestedAt = time.Now().Format(time.RFC3339Nano)
				break
			}
		}
	})

	log.Printf("[RedirectTest-Done] 渠道 %s 重定向验证完成，成功: %d/%d", channel.Name, countRedirectSuccesses(results), len(results))
	return results
}

func countRedirectSuccesses(results []RedirectModelResult) int {
	n := 0
	for _, r := range results {
		if r.Success {
			n++
		}
	}
	return n
}

func filterCapabilityProbeModels(probeModels []string, userModels []string) []string {
	if len(userModels) == 0 {
		return probeModels
	}
	userSet := make(map[string]bool, len(userModels))
	for _, model := range userModels {
		userSet[model] = true
	}
	filtered := make([]string, 0, len(userModels))
	for _, model := range probeModels {
		if userSet[model] {
			filtered = append(filtered, model)
		}
	}
	return filtered
}

// executeRedirectModelTest 单个重定向模型测试：用 actualModel 构建请求，probeModel 记录到结果
func executeRedirectModelTest(ctx context.Context, channel *config.UpstreamConfig, protocol, probeModel, actualModel string, timeout time.Duration, jobID string, cfgManager *config.ConfigManager, channelID int, apiKey string, channelLogStore *metrics.ChannelLogStore) RedirectModelResult {
	startedAt := time.Now()
	result := RedirectModelResult{
		ProbeModel:  probeModel,
		ActualModel: actualModel,
		StartedAt:   startedAt.Format(time.RFC3339Nano),
	}

	req, err := buildTestRequestWithModel(protocol, channel, actualModel)
	if err != nil {
		errMsg := fmt.Sprintf("build_request_failed: %v", err)
		result.Error = &errMsg
		result.TestedAt = time.Now().Format(time.RFC3339Nano)
		log.Printf("[RedirectTest-Model] 渠道 %s 构建请求失败 (探测: %s → 实际: %s): %v", channel.Name, probeModel, actualModel, err)
		return result
	}

	reqCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	req = req.WithContext(reqCtx)

	startTime := time.Now()
	log.Printf("[RedirectTest-Model] 渠道 %s 启动重定向测试 (探测: %s → 实际: %s)", channel.Name, probeModel, actualModel)

	success, streamingSupported, statusCode, respBody, sendErr := sendAndCheckStream(reqCtx, channel, req, protocol)
	result.Latency = time.Since(startTime).Milliseconds()
	result.TestedAt = time.Now().Format(time.RFC3339Nano)

	baseURL := req.URL.String()
	recordLog := func(success bool, statusCode int, errorInfo string) {
		common.RecordChannelLogWithSource(
			channelLogStore,
			channelID,
			actualModel, // 渠道日志记录重定向后的模型
			"",
			statusCode,
			result.Latency,
			success,
			apiKey,
			baseURL,
			errorInfo,
			protocol,
			false,
			metrics.RequestSourceCapabilityTest,
		)
	}

	// 拉黑判定：复用现有策略
	if !success && cfgManager != nil && apiKey != "" && respBody != nil {
		blacklistResult := common.ShouldBlacklistKey(statusCode, respBody)
		if blacklistResult.ShouldBlacklist {
			isBalanceError := blacklistResult.Reason == "insufficient_balance"
			if !isBalanceError || channel.IsAutoBlacklistBalanceEnabled() {
				apiType := channelKindToApiType(protocol)
				log.Printf("[RedirectTest-Blacklist] 渠道 %s 触发 Key 拉黑 (探测: %s → 实际: %s, 原因: %s, 状态码: %d)",
					channel.Name, probeModel, actualModel, blacklistResult.Reason, statusCode)
				if err := cfgManager.BlacklistKey(apiType, channelID, apiKey, blacklistResult.Reason, blacklistResult.Message); err != nil {
					log.Printf("[RedirectTest-Blacklist] 拉黑 Key 失败: %v", err)
				}
			}
		}
	}

	if success {
		result.Success = true
		result.StreamingSupported = streamingSupported
		recordLog(true, statusCode, "")
		log.Printf("[RedirectTest-Model] 渠道 %s 重定向测试成功 (探测: %s → 实际: %s, 流式: %v, 耗时: %dms)",
			channel.Name, probeModel, actualModel, streamingSupported, result.Latency)
		return result
	}

	errMsg := classifyError(sendErr, statusCode, reqCtx)
	if len(respBody) > 0 {
		errMsg = string(respBody)
	}
	errMsg = truncateCapabilityError(errMsg)
	result.Error = &errMsg
	recordLog(false, statusCode, errMsg)
	log.Printf("[RedirectTest-Model] 渠道 %s 重定向测试失败 (探测: %s → 实际: %s, 耗时: %dms): %s",
		channel.Name, probeModel, actualModel, result.Latency, errMsg)
	return result
}

// testProtocolCompatibility 并发测试多个协议的兼容性（已废弃，保留用于兼容）
func testProtocolCompatibility(ctx context.Context, channel *config.UpstreamConfig, protocols []string, timeout time.Duration, jobID string) []ProtocolTestResult {
	// 已废弃，直接调用新实现
	return runRoundRobinTests(ctx, channel, protocols, timeout, 10, jobID, nil, nil, nil, 0, "", "", "", nil)
}

// testSingleProtocol 已废弃，保留用于兼容
func testSingleProtocol(ctx context.Context, channel *config.UpstreamConfig, protocol string, timeout time.Duration, jobID string) ProtocolTestResult {
	// 已废弃，直接调用新实现
	results := runRoundRobinTests(ctx, channel, []string{protocol}, timeout, 10, jobID, nil, nil, nil, 0, "", "", "", nil)
	if len(results) > 0 {
		return results[0]
	}
	return ProtocolTestResult{Protocol: protocol, TestedAt: time.Now().Format(time.RFC3339)}
}

// testSingleModel 已废弃，保留用于兼容
func testSingleModel(ctx context.Context, channel *config.UpstreamConfig, protocol, model string, timeout time.Duration, jobID string) ModelTestResult {
	// 已废弃，直接调用 executeModelTest
	return executeModelTest(ctx, channel, protocol, model, timeout, jobID, nil, 0, "", "", nil)
}

func updateCapabilityJobModelResultsByActualModel(job *CapabilityTestJob, protocol, actualModel string, status CapabilityModelStatus, result ModelTestResult) int {
	if actualModel == "" {
		return 0
	}
	updated := 0
	for i := range job.Tests {
		if job.Tests[i].Protocol != protocol {
			continue
		}
		for _, modelResult := range job.Tests[i].ModelResults {
			modelActual := modelResult.ActualModel
			if modelActual == "" {
				modelActual = modelResult.Model
			}
			if modelActual != actualModel {
				continue
			}
			perModelResult := result
			perModelResult.Model = modelResult.Model
			perModelResult.ActualModel = actualModel
			updateCapabilityJobModelResult(job, protocol, modelResult.Model, status, perModelResult)
			updated++
		}
		return updated
	}
	return 0
}

func updateCapabilityJobModelResult(job *CapabilityTestJob, protocol, model string, status CapabilityModelStatus, result ModelTestResult) {
	for i := range job.Tests {
		if job.Tests[i].Protocol != protocol {
			continue
		}
		for j := range job.Tests[i].ModelResults {
			if job.Tests[i].ModelResults[j].Model != model {
				continue
			}
			job.Tests[i].ModelResults[j].Status = status
			job.Tests[i].ModelResults[j].ActualModel = result.ActualModel
			job.Tests[i].ModelResults[j].Success = result.Success
			job.Tests[i].ModelResults[j].Latency = result.Latency
			job.Tests[i].ModelResults[j].StreamingSupported = result.StreamingSupported
			job.Tests[i].ModelResults[j].Error = result.Error
			job.Tests[i].ModelResults[j].StartedAt = result.StartedAt
			job.Tests[i].ModelResults[j].TestedAt = result.TestedAt
			switch status {
			case CapabilityModelStatusQueued:
				job.Tests[i].ModelResults[j].Lifecycle = CapabilityLifecyclePending
				job.Tests[i].ModelResults[j].Outcome = CapabilityOutcomeUnknown
				job.Tests[i].ModelResults[j].Reason = nil
			case CapabilityModelStatusRunning:
				job.Tests[i].ModelResults[j].Lifecycle = CapabilityLifecycleActive
				job.Tests[i].ModelResults[j].Outcome = CapabilityOutcomeUnknown
				job.Tests[i].ModelResults[j].Reason = nil
			case CapabilityModelStatusSuccess:
				job.Tests[i].ModelResults[j].Lifecycle = CapabilityLifecycleDone
				job.Tests[i].ModelResults[j].Outcome = CapabilityOutcomeSuccess
				job.Tests[i].ModelResults[j].Reason = nil
			case CapabilityModelStatusFailed:
				job.Tests[i].ModelResults[j].Lifecycle = CapabilityLifecycleDone
				job.Tests[i].ModelResults[j].Outcome = CapabilityOutcomeFailed
				job.Tests[i].ModelResults[j].Reason = result.Error
			case CapabilityModelStatusSkipped:
				job.Tests[i].ModelResults[j].Lifecycle = CapabilityLifecycleDone
				job.Tests[i].ModelResults[j].Outcome = CapabilityOutcomeUnknown
				reason := "not_run"
				if result.Error != nil && *result.Error == "cancelled" {
					job.Tests[i].ModelResults[j].Lifecycle = CapabilityLifecycleCancelled
					job.Tests[i].ModelResults[j].Outcome = CapabilityOutcomeCancelled
					reason = "cancelled"
				}
				job.Tests[i].ModelResults[j].Reason = &reason
			}
			return
		}
	}
}

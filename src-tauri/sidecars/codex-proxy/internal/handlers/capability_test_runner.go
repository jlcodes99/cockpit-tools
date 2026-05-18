package handlers

import (
	"context"
	"fmt"
	"log"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/handlers/common"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
)

// ============== 核心测试逻辑 ==============

// testItem 代表一个 (协议, 模型) 测试单元
type testItem struct {
	protocol string
	model    string
	index    int // 模型在其协议列表中的索引
}

// buildRoundRobinQueue 构建交错队列
// 输出: messages[0], chat[0], gemini[0], responses[0], messages[1], chat[1], ...
func shouldRunRedirectVerification(protocols []string, sourceTab, channelServiceType string) bool {
	if sourceTab == "" {
		return false
	}
	virtualProtocol := sourceTab + "->" + channelServiceType
	for _, protocol := range protocols {
		if protocol == virtualProtocol {
			return true
		}
	}
	return false
}

func buildRoundRobinQueue(protocolModels map[string][]string, protocols []string) []testItem {
	maxModels := 0
	for _, models := range protocolModels {
		if len(models) > maxModels {
			maxModels = len(models)
		}
	}

	queue := make([]testItem, 0)
	for round := 0; round < maxModels; round++ {
		for _, protocol := range protocols {
			models := protocolModels[protocol]
			if round < len(models) {
				queue = append(queue, testItem{
					protocol: protocol,
					model:    models[round],
					index:    round,
				})
			}
		}
	}
	return queue
}

func runCapabilityTestJob(jobID, channelKind string, channelID int, channel config.UpstreamConfig, protocols []string, timeout time.Duration, effectiveRPM int, cacheKey, lookupKey, dispatcherKey string, previousResults map[string]map[string]ModelTestResult, userModels []string, sourceTab string, cfgManager *config.ConfigManager, channelLogStore *metrics.ChannelLogStore) {
	modelMappingHash := hashModelMapping(channel.ModelMapping)
	executionKey := buildCapabilityExecutionLookupKey(dispatcherKey, channelKind, protocols, userModels, modelMappingHash)
	// 创建可取消的 context，用于支持前端取消操作
	ctx, cancel := context.WithCancel(context.Background())
	capabilityJobs.setCancelFunc(jobID, cancel)

	// 检查是否在 queued 期间已被取消
	if ctx.Err() != nil {
		if lookupKey != "" {
			capabilityJobs.clearLookupKey(lookupKey)
		}
		capabilityJobs.clearLookupKey(executionKey)
		return
	}

	updatedJob, _ := capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
		if job.Lifecycle == CapabilityLifecycleCancelled {
			return
		}
		job.Lifecycle = CapabilityLifecycleActive
		job.Outcome = CapabilityOutcomeUnknown
		job.Status = deriveCapabilityJobStatus(job.Lifecycle, job.Outcome)
		job.StartedAt = time.Now().Format(time.RFC3339Nano)
		if job.RunMode == "" {
			job.RunMode = CapabilityRunModeFresh
		}
	})

	if updatedJob != nil && updatedJob.Lifecycle == CapabilityLifecycleCancelled {
		log.Printf("[CapabilityTest-Job] 任务 %s 在 queued 期间已被取消，跳过执行", jobID)
		if lookupKey != "" {
			capabilityJobs.clearLookupKey(lookupKey)
		}
		capabilityJobs.clearLookupKey(executionKey)
		return
	}

	log.Printf("[CapabilityTest-Job] 开始执行能力测试任务 %s，渠道 %s (ID:%d, 类型:%s)，协议: %v", jobID, channel.Name, channelID, channel.ServiceType, protocols)

	totalStart := time.Now()
	apiKey := ""
	if len(channel.APIKeys) > 0 {
		apiKey = channel.APIKeys[0]
	} else if len(channel.DisabledAPIKeys) > 0 {
		apiKey = channel.DisabledAPIKeys[0].Key
	}

	channelServiceType := serviceTypeToChannelKind(channel.ServiceType)

	// 并发运行重定向验证（仅在本次目标协议需要 sourceTab 虚拟协议时启动）
	// 通过 buffered channel 传递结果，避免共享变量在 cancel+超时场景下产生数据竞争：
	// - goroutine 单向写入 channel（容量 1，即便主协程放弃等待也不会阻塞）
	// - 主协程仅通过 channel 接收赋值，与 goroutine 无共享内存写入
	var redirectResults []RedirectModelResult
	var redirectCh chan []RedirectModelResult
	if shouldRunRedirectVerification(protocols, sourceTab, channelServiceType) {
		redirectCh = make(chan []RedirectModelResult, 1)
		go func() {
			redirectCh <- runRedirectVerification(ctx, &channel, channelKind, sourceTab, timeout, effectiveRPM, jobID, cfgManager, channelID, apiKey, dispatcherKey, channelLogStore, userModels)
		}()
	}

	// 过滤掉虚拟协议（含 "->"），原生协议测试只测基础协议
	nativeProtocols := make([]string, 0, len(protocols))
	for _, p := range protocols {
		if !strings.Contains(p, "->") {
			nativeProtocols = append(nativeProtocols, p)
		}
	}

	var results []ProtocolTestResult
	if len(nativeProtocols) > 0 {
		results = runRoundRobinTests(ctx, &channel, nativeProtocols, timeout, effectiveRPM, jobID, previousResults, userModels, cfgManager, channelID, channelKind, apiKey, dispatcherKey, channelLogStore)
	}

	// 等待重定向协程结果：优先直接等待；若任务被取消，给 2s 宽限后立即返回，
	// 避免阻塞在正在排队的限流槽位或未完成的 HTTP 请求上。超时丢弃则 redirectResults 保持 nil。
	if redirectCh != nil {
		select {
		case redirectResults = <-redirectCh:
		case <-ctx.Done():
			select {
			case redirectResults = <-redirectCh:
			case <-time.After(2 * time.Second):
				log.Printf("[RedirectTest-Cancel] 任务取消后 2s 内重定向验证协程未结束，放弃等待以加速退出 (jobID=%s)", jobID)
			}
		}
	}
	totalDuration := time.Since(totalStart).Milliseconds()

	compatible := make([]string, 0)
	for _, r := range results {
		if r.Success {
			compatible = append(compatible, r.Protocol)
		}
	}

	// 将 redirectResults 转换为虚拟协议测试结果
	var virtualResults []ProtocolTestResult
	if len(redirectResults) > 0 && sourceTab != "" {
		virtualProtocol := sourceTab + "->" + channelServiceType

		// 构建 actualModel 到测试结果的映射
		actualModelResults := make(map[string]RedirectModelResult)
		for _, rr := range redirectResults {
			actualModelResults[rr.ActualModel] = rr
		}

		// 获取本次探测模型，按顺序生成模型结果
		probeModels, _ := getCapabilityProbeModels(sourceTab)
		probeModels = filterCapabilityProbeModels(probeModels, userModels)
		var modelResults []ModelTestResult
		successCount := 0
		totalLatency := int64(0)
		hasStreaming := false

		for _, probeModel := range probeModels {
			actualModel := config.RedirectModel(probeModel, &channel)

			rr, tested := actualModelResults[actualModel]
			if tested {
				modelResults = append(modelResults, ModelTestResult{
					Model:              probeModel,
					ActualModel:        actualModel,
					Success:            rr.Success,
					Latency:            rr.Latency,
					StreamingSupported: rr.StreamingSupported,
					Error:              rr.Error,
					StartedAt:          rr.StartedAt,
					TestedAt:           rr.TestedAt,
				})
				if rr.Success {
					successCount++
					totalLatency += rr.Latency
					if rr.StreamingSupported {
						hasStreaming = true
					}
				}
			}
		}

		if len(modelResults) > 0 {
			avgLatency := int64(0)
			if successCount > 0 {
				avgLatency = totalLatency / int64(successCount)
			}

			virtualResult := ProtocolTestResult{
				Protocol:           virtualProtocol,
				Success:            successCount > 0,
				Latency:            avgLatency,
				StreamingSupported: hasStreaming,
				TestedModel:        "",
				ModelResults:       modelResults,
				SuccessCount:       successCount,
				AttemptedModels:    len(modelResults),
				TestedAt:           time.Now().Format(time.RFC3339),
			}

			if successCount == 0 {
				errMsg := "all_models_failed"
				virtualResult.Error = &errMsg
			} else {
				virtualResult.TestedModel = modelResults[0].ActualModel
			}

			virtualResults = append(virtualResults, virtualResult)

			if successCount > 0 {
				compatible = append(compatible, virtualProtocol)
			}
		}
	}

	// 将虚拟协议结果插入到结果列表开头
	if len(virtualResults) > 0 {
		results = append(virtualResults, results...)
	}

	resp := CapabilityTestResponse{
		ChannelID:           channelID,
		ChannelName:         channel.Name,
		SourceType:          channel.ServiceType,
		Tests:               results,
		RedirectTests:       redirectResults,
		CompatibleProtocols: compatible,
		TotalDuration:       totalDuration,
	}

	// 编排器已在执行过程中通过 capabilityJobs.update 实时维护 job.Tests，
	// 这里只更新最终元数据，不重建 Tests（避免覆盖编排器维护的 skipped 等中间状态）
	capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
		if job.Lifecycle == CapabilityLifecycleCancelled {
			job.TotalDuration = totalDuration
			job.RedirectTests = append([]RedirectModelResult(nil), redirectResults...)
			return
		}
		job.ChannelName = channel.Name
		job.SourceType = channel.ServiceType
		job.IdentityKey = dispatcherKey
		job.ExecutionKey = buildCapabilityExecutionLookupKey(dispatcherKey, channelKind, protocols, userModels, modelMappingHash)
		job.CompatibleProtocols = append([]string(nil), compatible...)
		job.RedirectTests = append([]RedirectModelResult(nil), redirectResults...)
		job.TotalDuration = totalDuration
		job.FinishedAt = time.Now().Format(time.RFC3339Nano)

		// 合并虚拟协议结果到 job.Tests（包含 modelResults）
		if len(virtualResults) > 0 {
			virtualJobResults := capabilityProtocolResultsFromResponse(CapabilityTestResponse{Tests: virtualResults})
			for _, vr := range virtualJobResults {
				found := false
				for i, existing := range job.Tests {
					if existing.Protocol == vr.Protocol {
						job.Tests[i] = vr
						found = true
						break
					}
				}
				if !found {
					job.Tests = append(job.Tests, vr)
				}
			}
		}
	})

	// 仅在未被取消且有兼容协议时写入缓存
	if len(compatible) > 0 && ctx.Err() == nil {
		setCapabilityCache(cacheKey, resp)
		log.Printf("[CapabilityTest-Cache] 渠道 %s (ID:%d) 写入缓存，兼容协议: %v", channel.Name, channelID, compatible)
	}

	// 取消时保留 lookupKey，允许后续恢复进度
	if lookupKey != "" && ctx.Err() == nil {
		capabilityJobs.clearLookupKey(lookupKey)
	}
	if ctx.Err() == nil {
		capabilityJobs.clearLookupKey(executionKey)
	}

	log.Printf("[CapabilityTest-Job] 能力测试任务 %s 完成，渠道 %s，兼容协议: %v，总耗时: %dms", jobID, channel.Name, compatible, totalDuration)
}

// runRoundRobinTests 核心编排器，串行按 round-robin 顺序逐个调度
// 所有模型都会被测试，不会在首次成功后跳过后续模型
// previousResults 可选：上次测试中成功的结果，跳过这些模型
func runRoundRobinTests(ctx context.Context, channel *config.UpstreamConfig, protocols []string, perModelTimeout time.Duration, effectiveRPM int, jobID string, previousResults map[string]map[string]ModelTestResult, userModels []string, cfgManager *config.ConfigManager, channelID int, channelKind, apiKey, dispatcherKey string, channelLogStore *metrics.ChannelLogStore) []ProtocolTestResult {
	// 1. 收集各协议模型列表，初始化 job 状态
	protocolModels := make(map[string][]string)
	protocolTimedOut := make(map[string]bool) // true = 全局超时强制终止
	results := make(map[string]*ProtocolTestResult)

	for _, protocol := range protocols {
		var models []string
		var err error
		if len(userModels) > 0 {
			// 用户指定模型列表，所有协议共用
			models = userModels
		} else {
			models, err = getCapabilityProbeModels(protocol)
		}
		if err != nil {
			errMsg := "no_models_configured"
			results[protocol] = &ProtocolTestResult{
				Protocol: protocol,
				Error:    &errMsg,
				TestedAt: time.Now().Format(time.RFC3339),
			}
			protocolTimedOut[protocol] = true // 无模型配置，视为已终止
			capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
				for i := range job.Tests {
					if job.Tests[i].Protocol == protocol {
						job.Tests[i].Lifecycle = CapabilityLifecycleDone
						job.Tests[i].Outcome = CapabilityOutcomeFailed
						job.Tests[i].Reason = &errMsg
						job.Tests[i].Error = &errMsg
						job.Tests[i].Status = deriveCapabilityProtocolStatus(job.Tests[i].Lifecycle, job.Tests[i].Outcome)
						job.Tests[i].TestedAt = time.Now().Format(time.RFC3339Nano)
						break
					}
				}
			})
			log.Printf("[CapabilityTest-Protocol] 渠道 %s 获取 %s 协议测试模型失败: %v", channel.Name, protocol, err)
			continue
		}

		protocolModels[protocol] = models
		results[protocol] = &ProtocolTestResult{
			Protocol:        protocol,
			TestedAt:        time.Now().Format(time.RFC3339),
			AttemptedModels: len(models),
			ModelResults:    make([]ModelTestResult, len(models)),
		}

		// 初始化协议状态和模型列表
		capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
			for i := range job.Tests {
				if job.Tests[i].Protocol == protocol {
					job.Tests[i].Status = CapabilityProtocolStatusQueued
					job.Tests[i].Lifecycle = CapabilityLifecyclePending
					job.Tests[i].Outcome = CapabilityOutcomeUnknown
					job.Tests[i].Reason = nil
					job.Tests[i].AttemptedModels = len(models)
					job.Tests[i].ModelResults = make([]CapabilityModelJobResult, len(models))
					for idx, modelName := range models {
						job.Tests[i].ModelResults[idx] = CapabilityModelJobResult{
							Model:     modelName,
							Status:    CapabilityModelStatusQueued,
							Lifecycle: CapabilityLifecyclePending,
							Outcome:   CapabilityOutcomeUnknown,
						}
					}
					job.Tests[i].TestedAt = time.Now().Format(time.RFC3339Nano)
					break
				}
			}
		})
		log.Printf("[CapabilityTest-Protocol] 开始测试渠道 %s 的 %s 协议兼容性", channel.Name, protocol)
	}

	// 1.5 预填充上次成功的结果
	if len(previousResults) > 0 {
		for protocol, modelMap := range previousResults {
			models := protocolModels[protocol]
			if len(models) == 0 {
				continue
			}
			result := results[protocol]
			if result == nil {
				continue
			}
			for i, modelName := range models {
				if prevResult, ok := modelMap[modelName]; ok && prevResult.Success {
					result.ModelResults[i] = prevResult
					result.SuccessCount++
					if result.SuccessCount == 1 {
						result.TestedModel = prevResult.Model
						result.StreamingSupported = prevResult.StreamingSupported
					}
					// 更新 job 中对应模型状态
					capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
						updateCapabilityJobModelResult(job, protocol, modelName, CapabilityModelStatusSuccess, prevResult)
					})
				}
			}
		}
	}

	// 2. 构建交错队列（排除已有成功结果的模型）
	// 需要保留原始索引，因为 result.ModelResults 按原始顺序排列
	filteredProtocolModels := make(map[string][]string)
	originalIndexMap := make(map[string]map[string]int) // protocol -> model -> original index
	for protocol, models := range protocolModels {
		originalIndexMap[protocol] = make(map[string]int)
		var filtered []string
		for origIdx, model := range models {
			if prevModels, ok := previousResults[protocol]; ok {
				if prevResult, ok := prevModels[model]; ok && prevResult.Success {
					continue // 跳过已成功的模型
				}
			}
			originalIndexMap[protocol][model] = origIdx
			filtered = append(filtered, model)
		}
		filteredProtocolModels[protocol] = filtered
	}
	queue := buildRoundRobinQueue(filteredProtocolModels, protocols)

	// 修正 queue 中的 index 为原始列表中的索引
	for i := range queue {
		if idxMap, ok := originalIndexMap[queue[i].protocol]; ok {
			if origIdx, ok := idxMap[queue[i].model]; ok {
				queue[i].index = origIdx
			}
		}
	}

	// 3. 计算全局超时
	// 串行执行中每个模型最多耗时 max(interval, perModelTimeout)，累加所有模型 + 缓冲
	totalModels := len(queue)
	interval := time.Minute / time.Duration(effectiveRPM)
	if interval <= 0 {
		interval = time.Minute / 10
	}
	perModelBudget := interval
	if perModelTimeout > perModelBudget {
		perModelBudget = perModelTimeout
	}
	globalTimeout := time.Duration(totalModels)*perModelBudget + 10*time.Second
	globalCtx, globalCancel := context.WithTimeout(ctx, globalTimeout)
	defer globalCancel()

	// 4. 逐项执行（所有模型都测，不早退）
	protocolStartTime := make(map[string]time.Time)
	protocolEndTime := make(map[string]time.Time)
	for _, item := range queue {
		// 检查全局超时
		if globalCtx.Err() != nil {
			log.Printf("[CapabilityTest-RoundRobin] 全局超时，终止测试")
			protocolTimedOut[item.protocol] = true
			break
		}

		// 记录协议首次测试时间
		if _, ok := protocolStartTime[item.protocol]; !ok {
			protocolStartTime[item.protocol] = time.Now()
		}

		// AcquireSendSlot（限流）
		if err := GetCapabilityTestDispatcher(dispatcherKey).AcquireSendSlot(globalCtx, interval); err != nil {
			log.Printf("[CapabilityTest-RoundRobin] 获取发送槽位失败: %v", err)
			break
		}

		// executeModelTest（单模型测试）
		modelResult := executeModelTest(globalCtx, channel, item.protocol, item.model, perModelTimeout, jobID, cfgManager, channelID, channelKind, apiKey, channelLogStore)
		result := results[item.protocol]
		result.ModelResults[item.index] = modelResult
		protocolEndTime[item.protocol] = time.Now() // 每次模型完成时更新协议结束时间

		if modelResult.Success {
			result.SuccessCount++
			// 首个成功模型：记录代表性字段，但继续测其余模型
			if result.SuccessCount == 1 {
				result.TestedModel = modelResult.Model
				result.StreamingSupported = modelResult.StreamingSupported
				result.Latency = protocolEndTime[item.protocol].Sub(protocolStartTime[item.protocol]).Milliseconds()
				log.Printf("[CapabilityTest-Protocol] 渠道 %s 的 %s 协议首个成功模型: %s (耗时: %dms)",
					channel.Name, item.protocol, result.TestedModel, result.Latency)
			}
		}
	}

	// 5. 收尾：标记残留 queued 模型为 skipped（仅超时时出现），更新协议最终状态
	for protocol, result := range results {
		models := protocolModels[protocol]

		// 回填未被调度到的模型（超时导致）为 skipped
		for i := range result.ModelResults {
			if result.ModelResults[i].Model == "" && i < len(models) {
				result.ModelResults[i] = ModelTestResult{
					Model:    models[i],
					Success:  false,
					Skipped:  true,
					TestedAt: time.Now().Format(time.RFC3339),
				}
				capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
					updateCapabilityJobModelResult(job, protocol, models[i], CapabilityModelStatusSkipped, result.ModelResults[i])
				})
			}
		}

		// 计算延迟：用协议的实际开始/结束时间，避免被其他协议的执行时间污染
		if startTime, ok := protocolStartTime[protocol]; ok {
			if endTime, ok := protocolEndTime[protocol]; ok {
				result.Latency = endTime.Sub(startTime).Milliseconds()
			} else {
				result.Latency = time.Since(startTime).Milliseconds()
			}
		}

		// 判断是否有实际测试过（至少有一个非 skipped 模型）
		hasTestedModel := false
		for _, mr := range result.ModelResults {
			if !mr.Skipped && mr.Model != "" {
				hasTestedModel = true
				break
			}
		}

		if result.SuccessCount > 0 {
			// 有至少一个成功模型
			result.Success = true
			capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
				for i := range job.Tests {
					if job.Tests[i].Protocol == protocol {
						job.Tests[i].Status = CapabilityProtocolStatusCompleted
						job.Tests[i].Success = true
						job.Tests[i].Latency = result.Latency
						job.Tests[i].StreamingSupported = result.StreamingSupported
						job.Tests[i].TestedModel = result.TestedModel
						job.Tests[i].SuccessCount = result.SuccessCount
						job.Tests[i].Error = nil
						job.Tests[i].TestedAt = time.Now().Format(time.RFC3339Nano)
						break
					}
				}
			})
			log.Printf("[CapabilityTest-Protocol] 渠道 %s 的 %s 协议测试完成 (成功: %d/%d, 耗时: %dms)",
				channel.Name, protocol, result.SuccessCount, result.AttemptedModels, result.Latency)
		} else if !hasTestedModel {
			// 协议完全未测试（超时）
			result.Success = false
			capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
				for i := range job.Tests {
					if job.Tests[i].Protocol == protocol {
						job.Tests[i].Status = CapabilityProtocolStatusFailed
						job.Tests[i].Success = false
						job.Tests[i].Latency = result.Latency
						job.Tests[i].SuccessCount = 0
						job.Tests[i].TestedAt = time.Now().Format(time.RFC3339Nano)
						for j := range job.Tests[i].ModelResults {
							if job.Tests[i].ModelResults[j].Status == CapabilityModelStatusQueued || job.Tests[i].ModelResults[j].Status == CapabilityModelStatusRunning {
								job.Tests[i].ModelResults[j].Status = CapabilityModelStatusSkipped
							}
						}
						break
					}
				}
			})
			log.Printf("[CapabilityTest-Protocol] 渠道 %s 的 %s 协议未实际测试（调度超时）", channel.Name, protocol)
		} else {
			// 全部模型测试失败
			result.Success = false
			errMsg := "all_models_failed"
			result.Error = &errMsg
			capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
				for i := range job.Tests {
					if job.Tests[i].Protocol == protocol {
						job.Tests[i].Status = CapabilityProtocolStatusFailed
						job.Tests[i].Success = false
						job.Tests[i].Latency = result.Latency
						job.Tests[i].SuccessCount = result.SuccessCount
						job.Tests[i].Error = result.Error
						job.Tests[i].TestedAt = time.Now().Format(time.RFC3339Nano)
						for j := range job.Tests[i].ModelResults {
							if job.Tests[i].ModelResults[j].Status == CapabilityModelStatusQueued || job.Tests[i].ModelResults[j].Status == CapabilityModelStatusRunning {
								job.Tests[i].ModelResults[j].Status = CapabilityModelStatusSkipped
							}
						}
						break
					}
				}
			})
			log.Printf("[CapabilityTest-Protocol] 渠道 %s 的 %s 协议全部模型测试失败 (尝试: %d, 总耗时: %dms)",
				channel.Name, protocol, result.AttemptedModels, result.Latency)
		}
	}

	// 6. 转换为有序结果
	orderedResults := make([]ProtocolTestResult, 0, len(protocols))
	for _, protocol := range protocols {
		if result, ok := results[protocol]; ok {
			orderedResults = append(orderedResults, *result)
		}
	}

	return orderedResults
}

func isCapabilityJobCancelled(jobID string) bool {
	job, ok := capabilityJobs.get(jobID)
	return ok && job.Lifecycle == CapabilityLifecycleCancelled
}

// executeModelTest 单模型测试（不调用 AcquireSendSlot，由编排器负责限流）
// 原生协议测试直接用原始模型名发请求，不走 ModelMapping 重定向
func executeModelTest(ctx context.Context, channel *config.UpstreamConfig, protocol, model string, timeout time.Duration, jobID string, cfgManager *config.ConfigManager, channelID int, channelKind, apiKey string, channelLogStore *metrics.ChannelLogStore) ModelTestResult {
	startedAt := time.Now()

	modelResult := ModelTestResult{
		Model:     model,
		StartedAt: startedAt.Format(time.RFC3339Nano),
	}

	req, err := buildTestRequestWithModel(protocol, channel, model)
	if err != nil {
		errMsg := fmt.Sprintf("build_request_failed: %v", err)
		modelResult.Error = &errMsg
		modelResult.TestedAt = time.Now().Format(time.RFC3339Nano)
		capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
			updateCapabilityJobModelResult(job, protocol, model, CapabilityModelStatusFailed, modelResult)
		})
		log.Printf("[CapabilityTest-Model] 渠道 %s 构建 %s 测试请求失败 (模型: %s): %v", channel.Name, protocol, model, err)
		return modelResult
	}

	reqCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	req = req.WithContext(reqCtx)

	capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
		if job.Lifecycle == CapabilityLifecycleCancelled {
			return
		}
		updateCapabilityJobModelResult(job, protocol, model, CapabilityModelStatusRunning, modelResult)
	})

	startTime := time.Now()
	log.Printf("[CapabilityTest-Model] 渠道 %s 启动 %s 协议模型测试 (模型: %s, startedAt: %s)",
		channel.Name, protocol, model, modelResult.StartedAt)
	success, streamingSupported, statusCode, respBody, sendErr := sendAndCheckStream(reqCtx, channel, req, protocol)
	modelResult.Latency = time.Since(startTime).Milliseconds()
	modelResult.TestedAt = time.Now().Format(time.RFC3339Nano)
	baseURL := req.URL.String()
	recordCapabilityTestLog := func(success bool, statusCode int, errorInfo string) {
		common.RecordChannelLogWithSource(
			channelLogStore,
			channelID,
			model,
			"",
			statusCode,
			modelResult.Latency,
			success,
			apiKey,
			baseURL,
			errorInfo,
			protocol,
			false,
			metrics.RequestSourceCapabilityTest,
		)
	}

	// 拉黑判定：非 2xx 响应时检查是否需要永久拉黑该 Key
	if !success && cfgManager != nil && apiKey != "" && respBody != nil {
		blacklistResult := common.ShouldBlacklistKey(statusCode, respBody)
		if blacklistResult.ShouldBlacklist {
			isBalanceError := blacklistResult.Reason == "insufficient_balance"
			if !isBalanceError || channel.IsAutoBlacklistBalanceEnabled() {
				apiType := channelKindToApiType(channelKind)
				log.Printf("[CapabilityTest-Blacklist] 渠道 %s 的 %s 协议触发 Key 拉黑 (模型: %s, 原因: %s, 状态码: %d)",
					channel.Name, protocol, model, blacklistResult.Reason, statusCode)
				if err := cfgManager.BlacklistKey(apiType, channelID, apiKey, blacklistResult.Reason, blacklistResult.Message); err != nil {
					log.Printf("[CapabilityTest-Blacklist] 拉黑 Key 失败: %v", err)
				}
			}
		}
	}

	if success {
		modelResult.Success = true
		modelResult.StreamingSupported = streamingSupported
		recordCapabilityTestLog(true, statusCode, "")
		capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
			updateCapabilityJobModelResult(job, protocol, model, CapabilityModelStatusSuccess, modelResult)
		})
		log.Printf("[CapabilityTest-Model] 渠道 %s 的 %s 协议测试成功 (模型: %s, 流式: %v, 耗时: %dms)",
			channel.Name, protocol, model, streamingSupported, modelResult.Latency)
		return modelResult
	}

	errMsg := classifyError(sendErr, statusCode, reqCtx)
	if len(respBody) > 0 {
		errMsg = string(respBody)
	}
	errMsg = truncateCapabilityError(errMsg)
	modelResult.Error = &errMsg
	recordCapabilityTestLog(false, statusCode, errMsg)
	capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
		if job.Lifecycle == CapabilityLifecycleCancelled {
			return
		}
		updateCapabilityJobModelResult(job, protocol, model, CapabilityModelStatusFailed, modelResult)
	})
	log.Printf("[CapabilityTest-Model] 渠道 %s 的 %s 协议测试失败 (模型: %s, 耗时: %dms): %s",
		channel.Name, protocol, model, modelResult.Latency, errMsg)
	return modelResult
}

func truncateCapabilityError(msg string) string {
	if len(msg) > 200 {
		return msg[:200]
	}
	return msg
}

package handlers

import (
	"log"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
	"github.com/gin-gonic/gin"
)

type CapabilityProtocolJobRef struct {
	JobID       string `json:"jobId"`
	ChannelKind string `json:"channelKind"`
	ChannelID   int    `json:"channelId"`
}

type CapabilitySnapshot struct {
	IdentityKey         string                              `json:"identityKey"`
	SourceType          string                              `json:"sourceType"`
	ProtocolJobIDs      map[string]string                   `json:"protocolJobIds,omitempty"`
	ProtocolJobRefs     map[string]CapabilityProtocolJobRef `json:"protocolJobRefs,omitempty"`
	Tests               []CapabilityProtocolJobResult       `json:"tests"`
	RedirectTests       []RedirectModelResult               `json:"redirectTests,omitempty"`
	CompatibleProtocols []string                            `json:"compatibleProtocols"`
	TotalDuration       int64                               `json:"totalDuration"`
	Progress            CapabilityTestJobProgress           `json:"progress"`
	Lifecycle           CapabilityLifecycle                 `json:"lifecycle"`
	Outcome             CapabilityOutcome                   `json:"outcome"`
	UpdatedAt           string                              `json:"updatedAt"`
}

type capabilitySnapshotStore struct {
	sync.RWMutex
	snapshots map[string]*CapabilitySnapshot
	ttl       time.Duration
}

const capabilitySnapshotTTL = 2 * time.Hour

var capabilitySnapshots = newCapabilitySnapshotStoreWithGC()

func newCapabilitySnapshotStore() *capabilitySnapshotStore {
	return &capabilitySnapshotStore{
		snapshots: make(map[string]*CapabilitySnapshot),
		ttl:       capabilitySnapshotTTL,
	}
}

func newCapabilitySnapshotStoreWithGC() *capabilitySnapshotStore {
	store := newCapabilitySnapshotStore()
	go store.gcLoop()
	return store
}

func cloneCapabilitySnapshot(snapshot *CapabilitySnapshot) *CapabilitySnapshot {
	if snapshot == nil {
		return nil
	}
	cloned := *snapshot
	cloned.ProtocolJobIDs = make(map[string]string, len(snapshot.ProtocolJobIDs))
	for protocol, jobID := range snapshot.ProtocolJobIDs {
		cloned.ProtocolJobIDs[protocol] = jobID
	}
	cloned.ProtocolJobRefs = make(map[string]CapabilityProtocolJobRef, len(snapshot.ProtocolJobRefs))
	for protocol, jobRef := range snapshot.ProtocolJobRefs {
		cloned.ProtocolJobRefs[protocol] = jobRef
	}
	cloned.Tests = make([]CapabilityProtocolJobResult, len(snapshot.Tests))
	for i, test := range snapshot.Tests {
		cloned.Tests[i] = test
		cloned.Tests[i].ModelResults = append([]CapabilityModelJobResult(nil), test.ModelResults...)
	}
	cloned.RedirectTests = append([]RedirectModelResult(nil), snapshot.RedirectTests...)
	cloned.CompatibleProtocols = append([]string(nil), snapshot.CompatibleProtocols...)
	return &cloned
}

func snapshotFromCapabilityJob(identityKey string, job *CapabilityTestJob) *CapabilitySnapshot {
	if job == nil {
		return nil
	}
	protocolJobIDs := make(map[string]string, len(job.Tests))
	protocolJobRefs := make(map[string]CapabilityProtocolJobRef, len(job.Tests))
	for _, test := range job.Tests {
		if test.Protocol == "" || job.JobID == "" {
			continue
		}
		protocolJobIDs[test.Protocol] = job.JobID
		protocolJobRefs[test.Protocol] = CapabilityProtocolJobRef{
			JobID:       job.JobID,
			ChannelKind: job.ChannelKind,
			ChannelID:   job.ChannelID,
		}
	}
	return &CapabilitySnapshot{
		IdentityKey:         identityKey,
		SourceType:          job.SourceType,
		ProtocolJobIDs:      protocolJobIDs,
		ProtocolJobRefs:     protocolJobRefs,
		Tests:               append([]CapabilityProtocolJobResult(nil), job.Tests...),
		RedirectTests:       append([]RedirectModelResult(nil), job.RedirectTests...),
		CompatibleProtocols: append([]string(nil), job.CompatibleProtocols...),
		TotalDuration:       job.TotalDuration,
		Progress:            job.Progress,
		Lifecycle:           job.Lifecycle,
		Outcome:             job.Outcome,
		UpdatedAt:           job.UpdatedAt,
	}
}

func (s *capabilitySnapshotStore) update(identityKey string, updater func(snapshot *CapabilitySnapshot)) *CapabilitySnapshot {
	s.Lock()
	defer s.Unlock()

	snapshot, ok := s.snapshots[identityKey]
	if !ok {
		snapshot = &CapabilitySnapshot{IdentityKey: identityKey}
		s.snapshots[identityKey] = snapshot
	}

	updater(snapshot)
	if snapshot.UpdatedAt == "" {
		snapshot.UpdatedAt = time.Now().Format(time.RFC3339Nano)
	}
	return cloneCapabilitySnapshot(snapshot)
}

func (s *capabilitySnapshotStore) replaceFromJob(identityKey string, job *CapabilityTestJob) *CapabilitySnapshot {
	if job == nil {
		return nil
	}
	s.Lock()
	defer s.Unlock()

	existing, hasExisting := s.snapshots[identityKey]
	if !hasExisting {
		existing = &CapabilitySnapshot{
			IdentityKey:     identityKey,
			ProtocolJobIDs:  make(map[string]string),
			ProtocolJobRefs: make(map[string]CapabilityProtocolJobRef),
		}
		s.snapshots[identityKey] = existing
	}
	if existing.ProtocolJobIDs == nil {
		existing.ProtocolJobIDs = make(map[string]string)
	}
	if existing.ProtocolJobRefs == nil {
		existing.ProtocolJobRefs = make(map[string]CapabilityProtocolJobRef)
	}

	// 按协议合并 ProtocolJobIDs
	for _, test := range job.Tests {
		if test.Protocol == "" || job.JobID == "" {
			continue
		}
		existing.ProtocolJobIDs[test.Protocol] = job.JobID
		existing.ProtocolJobRefs[test.Protocol] = CapabilityProtocolJobRef{
			JobID:       job.JobID,
			ChannelKind: job.ChannelKind,
			ChannelID:   job.ChannelID,
		}
	}

	// 按协议合并 Tests：同协议以最新 job 数据覆盖
	mergedTests := make([]CapabilityProtocolJobResult, len(existing.Tests))
	copy(mergedTests, existing.Tests)
	for _, jobTest := range job.Tests {
		found := false
		for i, existingTest := range mergedTests {
			if existingTest.Protocol == jobTest.Protocol {
				mergedTests[i] = jobTest
				found = true
				break
			}
		}
		if !found {
			mergedTests = append(mergedTests, jobTest)
		}
	}
	existing.Tests = mergedTests

	existing.CompatibleProtocols = mergeSnapshotCompatibleProtocols(existing.Tests)
	existing.TotalDuration = maxInt64(existing.TotalDuration, job.TotalDuration)
	existing.Progress = mergeSnapshotProgress(existing.Tests)
	existing.Lifecycle = mergeSnapshotLifecycle(existing.Tests)
	existing.Outcome = mergeSnapshotOutcome(existing.Tests, existing.Lifecycle)
	existing.RedirectTests = append([]RedirectModelResult(nil), job.RedirectTests...)
	existing.SourceType = job.SourceType
	existing.UpdatedAt = job.UpdatedAt
	if existing.UpdatedAt == "" {
		existing.UpdatedAt = time.Now().Format(time.RFC3339Nano)
	}

	return cloneCapabilitySnapshot(existing)
}

func maxInt64(a, b int64) int64 {
	if a > b {
		return a
	}
	return b
}

func mergeSnapshotCompatibleProtocols(tests []CapabilityProtocolJobResult) []string {
	compatible := make([]string, 0)
	for i := range tests {
		if tests[i].Outcome == CapabilityOutcomeSuccess || tests[i].Outcome == CapabilityOutcomePartial {
			compatible = append(compatible, tests[i].Protocol)
		}
	}
	return compatible
}

func mergeSnapshotProgress(tests []CapabilityProtocolJobResult) CapabilityTestJobProgress {
	progress := CapabilityTestJobProgress{}
	for _, test := range tests {
		for _, modelResult := range test.ModelResults {
			progress.TotalModels++
			switch modelResult.Status {
			case CapabilityModelStatusQueued:
				progress.QueuedModels++
			case CapabilityModelStatusRunning:
				progress.RunningModels++
			case CapabilityModelStatusSuccess:
				progress.SuccessModels++
				progress.CompletedModels++
			case CapabilityModelStatusFailed:
				progress.FailedModels++
				progress.CompletedModels++
			case CapabilityModelStatusSkipped:
				progress.SkippedModels++
				progress.CompletedModels++
			}
		}
	}
	return progress
}

func mergeSnapshotLifecycle(tests []CapabilityProtocolJobResult) CapabilityLifecycle {
	allTerminal := true
	allCancelled := len(tests) > 0

	for _, test := range tests {
		if test.Lifecycle == CapabilityLifecycleActive {
			return CapabilityLifecycleActive
		}
		if test.Lifecycle == CapabilityLifecyclePending {
			allTerminal = false
		}
		if test.Lifecycle != CapabilityLifecycleCancelled {
			allCancelled = false
		}
	}

	if allCancelled {
		return CapabilityLifecycleCancelled
	}
	if !allTerminal {
		return CapabilityLifecyclePending
	}
	return CapabilityLifecycleDone
}

func mergeSnapshotOutcome(tests []CapabilityProtocolJobResult, lifecycle CapabilityLifecycle) CapabilityOutcome {
	switch lifecycle {
	case CapabilityLifecycleCancelled:
		return CapabilityOutcomeCancelled
	case CapabilityLifecycleActive, CapabilityLifecyclePending:
		anySuccess := false
		for i := range tests {
			if tests[i].Outcome == CapabilityOutcomeSuccess || tests[i].Outcome == CapabilityOutcomePartial {
				anySuccess = true
				break
			}
		}
		if anySuccess {
			return CapabilityOutcomePartial
		}
		return CapabilityOutcomeUnknown
	case CapabilityLifecycleDone:
		anyPartial := false
		anySuccess := false
		anyFailed := false
		for i := range tests {
			switch tests[i].Outcome {
			case CapabilityOutcomePartial:
				anyPartial = true
			case CapabilityOutcomeSuccess:
				anySuccess = true
			case CapabilityOutcomeFailed:
				anyFailed = true
			}
		}
		switch {
		case anyPartial:
			return CapabilityOutcomePartial
		case anySuccess:
			return CapabilityOutcomeSuccess
		case anyFailed:
			return CapabilityOutcomeFailed
		default:
			return CapabilityOutcomeUnknown
		}
	}
	return CapabilityOutcomeUnknown
}

func (s *capabilitySnapshotStore) get(identityKey string) (*CapabilitySnapshot, bool) {
	s.RLock()
	defer s.RUnlock()
	snapshot, ok := s.snapshots[identityKey]
	if !ok {
		return nil, false
	}
	return cloneCapabilitySnapshot(snapshot), true
}

func (s *capabilitySnapshotStore) gcLoop() {
	ticker := time.NewTicker(30 * time.Minute)
	defer ticker.Stop()
	for range ticker.C {
		s.gc()
	}
}

func (s *capabilitySnapshotStore) gc() {
	cutoff := time.Now().Add(-s.ttl)
	s.Lock()
	defer s.Unlock()
	for identityKey, snapshot := range s.snapshots {
		if snapshot == nil {
			delete(s.snapshots, identityKey)
			continue
		}
		updatedAt, err := time.Parse(time.RFC3339Nano, snapshot.UpdatedAt)
		if err != nil || updatedAt.Before(cutoff) {
			delete(s.snapshots, identityKey)
		}
	}
}

func serviceTypeToChannelKind(serviceType string) string {
	switch serviceType {
	case "claude":
		return "messages"
	case "openai":
		return "chat"
	case "responses":
		return "responses"
	case "gemini":
		return "gemini"
	default:
		return "chat"
	}
}

func resolveCapabilityIdentityKey(channel *config.UpstreamConfig) string {
	if channel == nil {
		return ""
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
	return metrics.GenerateMetricsIdentityKey(baseURL, apiKey, channel.ServiceType)
}

func capabilityJobMatchesChannel(job *CapabilityTestJob, channel *config.UpstreamConfig, channelKind string, channelID int) bool {
	if job == nil {
		return false
	}
	if job.ChannelKind != channelKind {
		return false
	}
	if job.ChannelID == channelID {
		return true
	}
	if channel == nil {
		return false
	}
	identityKey := resolveCapabilityIdentityKey(channel)
	return identityKey != "" && job.IdentityKey == identityKey
}

func filterSameSourceVirtualProtocols(tests []CapabilityProtocolJobResult, preserveProtocol string) []CapabilityProtocolJobResult {
	if len(tests) == 0 {
		return tests
	}
	filtered := make([]CapabilityProtocolJobResult, 0, len(tests))
	for _, test := range tests {
		if parts := strings.SplitN(test.Protocol, "->", 2); len(parts) == 2 && parts[0] != "" && parts[0] == parts[1] && test.Protocol != preserveProtocol {
			continue
		}
		filtered = append(filtered, test)
	}
	return filtered
}

func GetCapabilitySnapshot(cfgManager *config.ConfigManager, channelKind string) gin.HandlerFunc {
	return func(c *gin.Context) {
		id, err := parseCapabilityChannelID(c)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid channel ID"})
			return
		}

		channel, getErr := getCapabilityTestChannel(cfgManager, channelKind, id)
		if getErr != nil {
			c.JSON(http.StatusNotFound, gin.H{"error": "channel not found"})
			return
		}

		// 获取 sourceTab 参数（从查询参数）
		sourceTab := c.Query("sourceTab")

		identityKey := resolveCapabilityIdentityKey(channel)
		snapshot, ok := capabilitySnapshots.get(identityKey)
		if !ok {
			// 如果没有快照，创建一个空快照，但包含虚拟协议占位符
			snapshot = &CapabilitySnapshot{
				IdentityKey:         identityKey,
				SourceType:          channel.ServiceType,
				ProtocolJobIDs:      make(map[string]string),
				ProtocolJobRefs:     make(map[string]CapabilityProtocolJobRef),
				Tests:               []CapabilityProtocolJobResult{},
				CompatibleProtocols: []string{},
				UpdatedAt:           time.Now().Format(time.RFC3339Nano),
			}
		}

		// 动态添加虚拟协议占位符（只基于用户选择的 sourceTab）
		// 使用渠道的实际 serviceType 作为目标协议
		channelServiceType := serviceTypeToChannelKind(channel.ServiceType)
		log.Printf("[Snapshot-Debug] sourceTab=%s, channelKind=%s, channelServiceType=%s", sourceTab, channelKind, channelServiceType)

		if sourceTab != "" {
			// 获取 sourceTab 协议的探测模型
			probeModels, err := getCapabilityProbeModels(sourceTab)
			if err == nil {
				// 检查是否至少有一个模型被重定向
				hasRedirect := false
				for _, m := range probeModels {
					actual := config.RedirectModel(m, channel)
					if actual != m {
						hasRedirect = true
						break
					}
				}

				// 只要有模型被重定向就生成虚拟协议占位符；同源场景表示纯模型重定向测试。
				if hasRedirect {
					virtualProtocol := sourceTab + "->" + channelServiceType
					// 检查快照中是否已经有这个虚拟协议
					foundIndex := -1
					for i, test := range snapshot.Tests {
						if test.Protocol == virtualProtocol {
							foundIndex = i
							break
						}
					}

					// 构建模型结果列表（包含所有探测模型，被重定向的标记 actualModel）
					buildModelResults := func(existingResults []CapabilityModelJobResult) []CapabilityModelJobResult {
						existingByModel := make(map[string]CapabilityModelJobResult, len(existingResults))
						for _, result := range existingResults {
							existingByModel[result.Model] = result
						}

						modelResults := make([]CapabilityModelJobResult, 0, len(probeModels))
						for _, m := range probeModels {
							actual := config.RedirectModel(m, channel)
							result, ok := existingByModel[m]
							if !ok {
								result = CapabilityModelJobResult{
									Model:  m,
									Status: "idle",
								}
							}
							if actual != m {
								result.ActualModel = actual
							} else if result.ActualModel == m {
								result.ActualModel = ""
							}
							modelResults = append(modelResults, result)
						}
						return modelResults
					}

					if foundIndex < 0 {
						// 没有找到，添加一个占位符
						log.Printf("[Snapshot-Debug] 添加虚拟协议占位符: %s", virtualProtocol)
						modelResults := buildModelResults(nil)
						if len(modelResults) > 0 {
							snapshot.Tests = append([]CapabilityProtocolJobResult{
								{
									Protocol:        virtualProtocol,
									Status:          "idle",
									Lifecycle:       CapabilityLifecycleDone,
									Outcome:         CapabilityOutcomeUnknown,
									ModelResults:    modelResults,
									AttemptedModels: len(modelResults),
									SuccessCount:    0,
								},
							}, snapshot.Tests...)
						}
					} else {
						// 已存在，始终用当前配置刷新模型列表
						// 这样配置变更（ModelMapping、ServiceType）会自动反映
						existing := &snapshot.Tests[foundIndex]
						modelResults := buildModelResults(existing.ModelResults)
						if len(modelResults) > 0 {
							existing.ModelResults = modelResults
							existing.AttemptedModels = len(modelResults)
						}
					}
				}
			}

			// 过滤掉其他非当前 sourceTab 的虚拟协议
			expectedVirtualProtocol := sourceTab + "->" + channelServiceType
			filteredTests := make([]CapabilityProtocolJobResult, 0, len(snapshot.Tests))
			for _, test := range snapshot.Tests {
				// 保留非虚拟协议（不含 "->" 的）或匹配当前 sourceTab 的虚拟协议
				if !strings.Contains(test.Protocol, "->") || test.Protocol == expectedVirtualProtocol {
					filteredTests = append(filteredTests, test)
				}
			}
			snapshot.Tests = filteredTests
		}

		// 剔除非当前 sourceTab 的同源虚拟协议，保留当前同源纯模型重定向测试。
		preserveVirtualProtocol := ""
		if sourceTab != "" {
			preserveVirtualProtocol = sourceTab + "->" + channelServiceType
		}
		snapshot.Tests = filterSameSourceVirtualProtocols(snapshot.Tests, preserveVirtualProtocol)

		c.JSON(http.StatusOK, snapshot)
	}
}

package handlers

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"log"
	"net/http"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

type CapabilityLifecycle string

type CapabilityOutcome string

type CapabilityRunMode string

type CapabilityJobStatus string

type CapabilityProtocolStatus string

type CapabilityModelStatus string

const (
	CapabilityLifecyclePending   CapabilityLifecycle = "pending"
	CapabilityLifecycleActive    CapabilityLifecycle = "active"
	CapabilityLifecycleDone      CapabilityLifecycle = "done"
	CapabilityLifecycleCancelled CapabilityLifecycle = "cancelled"
)

const (
	CapabilityOutcomeUnknown   CapabilityOutcome = "unknown"
	CapabilityOutcomeSuccess   CapabilityOutcome = "success"
	CapabilityOutcomeFailed    CapabilityOutcome = "failed"
	CapabilityOutcomePartial   CapabilityOutcome = "partial"
	CapabilityOutcomeCancelled CapabilityOutcome = "cancelled"
)

const (
	CapabilityRunModeFresh                CapabilityRunMode = "fresh"
	CapabilityRunModeReusedRunning        CapabilityRunMode = "reused_running"
	CapabilityRunModeResumedCancelled     CapabilityRunMode = "resumed_cancelled"
	CapabilityRunModeCacheHit             CapabilityRunMode = "cache_hit"
	CapabilityRunModeReusedPreviousResult CapabilityRunMode = "reused_previous_results"
)

const (
	CapabilityJobStatusQueued    CapabilityJobStatus = "queued"
	CapabilityJobStatusRunning   CapabilityJobStatus = "running"
	CapabilityJobStatusCompleted CapabilityJobStatus = "completed"
	CapabilityJobStatusFailed    CapabilityJobStatus = "failed"
	CapabilityJobStatusCancelled CapabilityJobStatus = "cancelled"
)

const (
	CapabilityProtocolStatusQueued    CapabilityProtocolStatus = "queued"
	CapabilityProtocolStatusRunning   CapabilityProtocolStatus = "running"
	CapabilityProtocolStatusCompleted CapabilityProtocolStatus = "completed"
	CapabilityProtocolStatusFailed    CapabilityProtocolStatus = "failed"
)

const (
	CapabilityModelStatusQueued  CapabilityModelStatus = "queued"
	CapabilityModelStatusRunning CapabilityModelStatus = "running"
	CapabilityModelStatusSuccess CapabilityModelStatus = "success"
	CapabilityModelStatusFailed  CapabilityModelStatus = "failed"
	CapabilityModelStatusSkipped CapabilityModelStatus = "skipped"
	CapabilityModelStatusIdle    CapabilityModelStatus = "idle"
)

type CapabilityTestJobProgress struct {
	TotalModels     int `json:"totalModels"`
	QueuedModels    int `json:"queuedModels"`
	RunningModels   int `json:"runningModels"`
	SuccessModels   int `json:"successModels"`
	FailedModels    int `json:"failedModels"`
	SkippedModels   int `json:"skippedModels"`
	CompletedModels int `json:"completedModels"`
}

type CapabilityModelJobResult struct {
	Model              string                `json:"model"`
	ActualModel        string                `json:"actualModel,omitempty"` // 经 ModelMapping 重定向后实际发送给上游的模型名
	Status             CapabilityModelStatus `json:"status"`
	Lifecycle          CapabilityLifecycle   `json:"lifecycle"`
	Outcome            CapabilityOutcome     `json:"outcome"`
	Reason             *string               `json:"reason,omitempty"`
	Success            bool                  `json:"success"`
	Latency            int64                 `json:"latency"`
	StreamingSupported bool                  `json:"streamingSupported"`
	Error              *string               `json:"error,omitempty"`
	StartedAt          string                `json:"startedAt,omitempty"`
	TestedAt           string                `json:"testedAt,omitempty"`
}

type CapabilityProtocolJobResult struct {
	Protocol           string                     `json:"protocol"`
	Status             CapabilityProtocolStatus   `json:"status"`
	Lifecycle          CapabilityLifecycle        `json:"lifecycle"`
	Outcome            CapabilityOutcome          `json:"outcome"`
	Reason             *string                    `json:"reason,omitempty"`
	Success            bool                       `json:"success"`
	Latency            int64                      `json:"latency"`
	StreamingSupported bool                       `json:"streamingSupported"`
	TestedModel        string                     `json:"testedModel"`
	ModelResults       []CapabilityModelJobResult `json:"modelResults,omitempty"`
	SuccessCount       int                        `json:"successCount,omitempty"`
	AttemptedModels    int                        `json:"attemptedModels,omitempty"`
	Error              *string                    `json:"error,omitempty"`
	TestedAt           string                     `json:"testedAt"`
}

// RedirectModelResult 单个探测模型经 ModelMapping 后的测试结果
type RedirectModelResult struct {
	ProbeModel         string  `json:"probeModel"`  // 原生探测模型名
	ActualModel        string  `json:"actualModel"` // ModelMapping 后实际发给上游的模型名
	Success            bool    `json:"success"`
	Latency            int64   `json:"latency"` // 毫秒
	StreamingSupported bool    `json:"streamingSupported,omitempty"`
	Error              *string `json:"error,omitempty"`
	StartedAt          string  `json:"startedAt,omitempty"`
	TestedAt           string  `json:"testedAt"`
}

type CapabilityTestJob struct {
	JobID               string                        `json:"jobId"`
	IdentityKey         string                        `json:"identityKey,omitempty"`
	ExecutionKey        string                        `json:"executionKey,omitempty"`
	ChannelID           int                           `json:"channelId"`
	ChannelName         string                        `json:"channelName"`
	ChannelKind         string                        `json:"channelKind"`
	SourceType          string                        `json:"sourceType"`
	Status              CapabilityJobStatus           `json:"status"`
	Lifecycle           CapabilityLifecycle           `json:"lifecycle"`
	Outcome             CapabilityOutcome             `json:"outcome"`
	Reason              *string                       `json:"reason,omitempty"`
	RunMode             CapabilityRunMode             `json:"runMode,omitempty"`
	SummaryReason       string                        `json:"summaryReason,omitempty"`
	ActiveOperations    int                           `json:"activeOperations,omitempty"`
	IsResumed           bool                          `json:"isResumed,omitempty"`
	HasReusedResults    bool                          `json:"hasReusedResults,omitempty"`
	Tests               []CapabilityProtocolJobResult `json:"tests"`
	RedirectTests       []RedirectModelResult         `json:"redirectTests,omitempty"`
	CompatibleProtocols []string                      `json:"compatibleProtocols"`
	TotalDuration       int64                         `json:"totalDuration"`
	StartedAt           string                        `json:"startedAt,omitempty"`
	UpdatedAt           string                        `json:"updatedAt"`
	FinishedAt          string                        `json:"finishedAt,omitempty"`
	Progress            CapabilityTestJobProgress     `json:"progress"`
	Error               *string                       `json:"error,omitempty"`
	CacheHit            bool                          `json:"cacheHit,omitempty"`
	TargetProtocols     []string                      `json:"targetProtocols,omitempty"`
	TimeoutMilliseconds int                           `json:"timeoutMilliseconds,omitempty"`
	EffectiveRPM        int                           `json:"effectiveRPM,omitempty"`
	CancelFunc          context.CancelFunc            `json:"-"`
}

type capabilityTestJobStore struct {
	sync.RWMutex
	jobs      map[string]*CapabilityTestJob
	lookupKey map[string]string
}

var capabilityJobs = newCapabilityTestJobStore()

func newCapabilityTestJobStore() *capabilityTestJobStore {
	s := &capabilityTestJobStore{
		jobs:      make(map[string]*CapabilityTestJob),
		lookupKey: make(map[string]string),
	}
	go s.gcLoop()
	return s
}

// gcLoop 定期清理已完成且超过 2 小时的 job，防止 job store 无限增长
func (s *capabilityTestJobStore) gcLoop() {
	ticker := time.NewTicker(30 * time.Minute)
	defer ticker.Stop()
	for range ticker.C {
		s.gc()
	}
}

func (s *capabilityTestJobStore) gc() {
	cutoff := time.Now().Add(-2 * time.Hour)
	s.Lock()
	defer s.Unlock()
	for jobID, job := range s.jobs {
		if job.Lifecycle != CapabilityLifecycleDone && job.Lifecycle != CapabilityLifecycleCancelled {
			continue
		}
		t, err := time.Parse(time.RFC3339Nano, job.UpdatedAt)
		if err != nil || t.Before(cutoff) {
			delete(s.jobs, jobID)
		}
	}
	log.Printf("[CapabilityTest-GC] job store 清理完成，当前 job 数: %d", len(s.jobs))
}

func newCapabilityJobID() string {
	b := make([]byte, 8)
	if _, err := rand.Read(b); err != nil {
		// 极低概率退化到时间戳
		return fmt.Sprintf("cap-%d", time.Now().UnixNano())
	}
	return "cap-" + hex.EncodeToString(b)
}

func newCapabilityTestJob(channelID int, channelName, channelKind, sourceType string, protocols []string, timeout time.Duration, effectiveRPM int) *CapabilityTestJob {
	return newCapabilityTestJobWithModels(channelID, channelName, channelKind, sourceType, protocols, timeout, effectiveRPM, nil)
}

func newCapabilityTestJobWithModels(channelID int, channelName, channelKind, sourceType string, protocols []string, timeout time.Duration, effectiveRPM int, requestedModels []string) *CapabilityTestJob {
	now := time.Now().Format(time.RFC3339Nano)
	job := &CapabilityTestJob{
		JobID:               newCapabilityJobID(),
		IdentityKey:         "",
		ExecutionKey:        "",
		ChannelID:           channelID,
		ChannelName:         channelName,
		ChannelKind:         channelKind,
		SourceType:          sourceType,
		Status:              CapabilityJobStatusQueued,
		Lifecycle:           CapabilityLifecyclePending,
		Outcome:             CapabilityOutcomeUnknown,
		RunMode:             CapabilityRunModeFresh,
		CompatibleProtocols: make([]string, 0),
		Tests:               make([]CapabilityProtocolJobResult, 0, len(protocols)),
		UpdatedAt:           now,
		TargetProtocols:     append([]string(nil), protocols...),
		TimeoutMilliseconds: int(timeout / time.Millisecond),
		EffectiveRPM:        effectiveRPM,
	}

	for _, protocol := range protocols {
		models := capabilityJobInitialModels(protocol, requestedModels)
		modelResults := make([]CapabilityModelJobResult, len(models))
		for i, model := range models {
			modelResults[i] = CapabilityModelJobResult{
				Model:     model,
				Status:    CapabilityModelStatusQueued,
				Lifecycle: CapabilityLifecyclePending,
				Outcome:   CapabilityOutcomeUnknown,
			}
		}
		job.Tests = append(job.Tests, CapabilityProtocolJobResult{
			Protocol:        protocol,
			Status:          CapabilityProtocolStatusQueued,
			Lifecycle:       CapabilityLifecyclePending,
			Outcome:         CapabilityOutcomeUnknown,
			AttemptedModels: len(modelResults),
			ModelResults:    modelResults,
			TestedAt:        now,
		})
	}

	return job
}

func capabilityJobInitialModels(protocol string, requestedModels []string) []string {
	if len(requestedModels) > 0 {
		return append([]string(nil), requestedModels...)
	}
	models, err := getCapabilityProbeModels(protocol)
	if err != nil {
		return nil
	}
	return models
}

func buildCapabilityJobLookupKey(cacheKey, channelKind string, channelID int) string {
	return fmt.Sprintf("%s:%s:%d", cacheKey, channelKind, channelID)
}

func buildCapabilityExecutionLookupKey(identityKey, channelKind string, protocols []string, models []string, modelMappingHash string) string {
	sortedProtocols := append([]string(nil), protocols...)
	sort.Strings(sortedProtocols)
	normalizedModels := normalizeCapabilityModels(models)
	key := fmt.Sprintf("%s:%s:%s:%s", identityKey, channelKind, strings.Join(sortedProtocols, ","), strings.Join(normalizedModels, ","))
	if modelMappingHash != "" {
		key += ":" + modelMappingHash
	}
	return key
}

func (s *capabilityTestJobStore) bindLookupKey(lookupKey, jobID string) {
	s.Lock()
	defer s.Unlock()
	s.lookupKey[lookupKey] = jobID
}

func (s *capabilityTestJobStore) clearLookupKey(lookupKey string) {
	s.Lock()
	defer s.Unlock()
	delete(s.lookupKey, lookupKey)
}

func (s *capabilityTestJobStore) getByLookupKey(lookupKey string) (*CapabilityTestJob, bool) {
	s.RLock()
	jobID, ok := s.lookupKey[lookupKey]
	s.RUnlock()
	if !ok {
		return nil, false
	}
	return s.get(jobID)
}

func (s *capabilityTestJobStore) create(job *CapabilityTestJob) {
	s.Lock()
	defer s.Unlock()
	cloned := cloneCapabilityTestJob(job)
	s.jobs[job.JobID] = cloned
	if cloned.IdentityKey != "" {
		capabilitySnapshots.replaceFromJob(cloned.IdentityKey, cloned)
	}
}

func (s *capabilityTestJobStore) get(jobID string) (*CapabilityTestJob, bool) {
	s.RLock()
	defer s.RUnlock()
	job, ok := s.jobs[jobID]
	if !ok {
		return nil, false
	}
	return cloneCapabilityTestJob(job), true
}

func (s *capabilityTestJobStore) getOrCreateByLookupKey(
	lookupKey string,
	builder func() *CapabilityTestJob,
) (*CapabilityTestJob, bool) {
	s.Lock()
	defer s.Unlock()

	if lookupKey != "" {
		if jobID, ok := s.lookupKey[lookupKey]; ok {
			if job, exists := s.jobs[jobID]; exists {
				return cloneCapabilityTestJob(job), true
			}
		}
	}

	job := builder()
	s.jobs[job.JobID] = cloneCapabilityTestJob(job)
	if lookupKey != "" {
		s.lookupKey[lookupKey] = job.JobID
	}
	return cloneCapabilityTestJob(job), false
}

func (s *capabilityTestJobStore) update(jobID string, updater func(job *CapabilityTestJob)) (*CapabilityTestJob, bool) {
	s.Lock()
	defer s.Unlock()
	job, ok := s.jobs[jobID]
	if !ok {
		return nil, false
	}
	updater(job)
	job.UpdatedAt = time.Now().Format(time.RFC3339Nano)
	recomputeCapabilityJob(job)
	cloned := cloneCapabilityTestJob(job)
	if cloned.IdentityKey != "" {
		capabilitySnapshots.replaceFromJob(cloned.IdentityKey, cloned)
	}
	return cloned, true
}

// setCancelFunc 直接设置内部 job 的 CancelFunc（不走 clone，因为 CancelFunc 不可复制）
func (s *capabilityTestJobStore) setCancelFunc(jobID string, cancel context.CancelFunc) {
	s.Lock()
	defer s.Unlock()
	if job, ok := s.jobs[jobID]; ok {
		job.CancelFunc = cancel
	}
}

// getCancelFunc 获取 job 的 CancelFunc
func (s *capabilityTestJobStore) getCancelFunc(jobID string) (context.CancelFunc, bool) {
	s.RLock()
	defer s.RUnlock()
	job, ok := s.jobs[jobID]
	if !ok || job.CancelFunc == nil {
		return nil, false
	}
	return job.CancelFunc, true
}

func cloneCapabilityTestJob(job *CapabilityTestJob) *CapabilityTestJob {
	if job == nil {
		return nil
	}
	cloned := *job
	cloned.Tests = make([]CapabilityProtocolJobResult, len(job.Tests))
	for i, test := range job.Tests {
		cloned.Tests[i] = test
		cloned.Tests[i].ModelResults = append([]CapabilityModelJobResult(nil), test.ModelResults...)
	}
	cloned.CompatibleProtocols = append([]string(nil), job.CompatibleProtocols...)
	cloned.TargetProtocols = append([]string(nil), job.TargetProtocols...)
	cloned.RedirectTests = append([]RedirectModelResult(nil), job.RedirectTests...)
	return &cloned
}

func recomputeCapabilityJob(job *CapabilityTestJob) {
	progress := CapabilityTestJobProgress{}
	compatible := make([]string, 0)

	if job.Lifecycle == CapabilityLifecycleCancelled {
		for _, test := range job.Tests {
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
		job.Progress = progress
		job.CompatibleProtocols = compatible
		job.ActiveOperations = 0
		job.Outcome = CapabilityOutcomeCancelled
		job.Status = deriveCapabilityJobStatus(job.Lifecycle, job.Outcome)
		if job.FinishedAt == "" {
			job.FinishedAt = job.UpdatedAt
		}
		return
	}
	allProtocolsTerminal := true
	anyProtocolActive := false
	allProtocolsCancelled := len(job.Tests) > 0
	anyProtocolFailed := false
	anyProtocolPartial := false
	anyProtocolSucceeded := false

	for i := range job.Tests {
		test := &job.Tests[i]
		recomputeCapabilityProtocol(test)

		if test.Outcome == CapabilityOutcomeSuccess || test.Outcome == CapabilityOutcomePartial {
			compatible = append(compatible, test.Protocol)
		}
		if test.Lifecycle == CapabilityLifecycleActive || test.Lifecycle == CapabilityLifecyclePending {
			allProtocolsTerminal = false
		}
		if test.Lifecycle == CapabilityLifecycleActive {
			anyProtocolActive = true
		}
		if test.Lifecycle != CapabilityLifecycleCancelled {
			allProtocolsCancelled = false
		}
		if test.Outcome == CapabilityOutcomeFailed {
			anyProtocolFailed = true
		}
		if test.Outcome == CapabilityOutcomePartial {
			anyProtocolPartial = true
		}
		if test.Outcome == CapabilityOutcomeSuccess {
			anyProtocolSucceeded = true
		}

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

	sort.Strings(compatible)
	job.Progress = progress
	job.CompatibleProtocols = compatible
	job.ActiveOperations = progress.RunningModels

	if job.StartedAt == "" && (progress.RunningModels > 0 || progress.CompletedModels > 0 || job.Lifecycle == CapabilityLifecycleDone) {
		job.StartedAt = job.UpdatedAt
	}

	if job.Lifecycle == CapabilityLifecycleCancelled {
		job.Outcome = CapabilityOutcomeCancelled
		if job.FinishedAt == "" {
			job.FinishedAt = job.UpdatedAt
		}
		job.Status = deriveCapabilityJobStatus(job.Lifecycle, job.Outcome)
		return
	}

	if anyProtocolActive || progress.RunningModels > 0 {
		job.Lifecycle = CapabilityLifecycleActive
		if anyProtocolSucceeded || anyProtocolPartial {
			job.Outcome = CapabilityOutcomePartial
		} else {
			job.Outcome = CapabilityOutcomeUnknown
		}
		job.Status = deriveCapabilityJobStatus(job.Lifecycle, job.Outcome)
		job.FinishedAt = ""
		return
	}

	if !allProtocolsTerminal {
		job.Lifecycle = CapabilityLifecyclePending
		job.Outcome = CapabilityOutcomeUnknown
		job.Status = deriveCapabilityJobStatus(job.Lifecycle, job.Outcome)
		job.FinishedAt = ""
		return
	}

	if allProtocolsCancelled && len(job.Tests) > 0 {
		job.Lifecycle = CapabilityLifecycleCancelled
		job.Outcome = CapabilityOutcomeCancelled
	} else {
		job.Lifecycle = CapabilityLifecycleDone
		switch {
		case anyProtocolPartial:
			job.Outcome = CapabilityOutcomePartial
		case anyProtocolSucceeded:
			job.Outcome = CapabilityOutcomeSuccess
		case anyProtocolFailed || progress.TotalModels > 0:
			job.Outcome = CapabilityOutcomeFailed
		default:
			job.Outcome = CapabilityOutcomeUnknown
		}
	}

	job.Status = deriveCapabilityJobStatus(job.Lifecycle, job.Outcome)
	if job.FinishedAt == "" {
		job.FinishedAt = job.UpdatedAt
	}
}

func recomputeCapabilityProtocol(test *CapabilityProtocolJobResult) {
	allTerminal := true
	allCancelled := len(test.ModelResults) > 0
	anyActive := false
	anySuccess := false
	anyFailed := false
	anyCancelled := false
	successCount := 0
	var firstSuccessModel string
	var firstSuccessStreaming bool

	for _, modelResult := range test.ModelResults {
		if modelResult.Lifecycle == CapabilityLifecyclePending || modelResult.Lifecycle == CapabilityLifecycleActive {
			allTerminal = false
		}
		if modelResult.Lifecycle == CapabilityLifecycleActive {
			anyActive = true
		}
		if modelResult.Lifecycle != CapabilityLifecycleCancelled {
			allCancelled = false
		}
		switch modelResult.Outcome {
		case CapabilityOutcomeSuccess:
			anySuccess = true
			successCount++
			if firstSuccessModel == "" {
				firstSuccessModel = modelResult.Model
				firstSuccessStreaming = modelResult.StreamingSupported
			}
		case CapabilityOutcomeFailed:
			anyFailed = true
		case CapabilityOutcomeCancelled:
			anyCancelled = true
		}
	}

	test.SuccessCount = successCount
	if firstSuccessModel != "" {
		test.TestedModel = firstSuccessModel
		test.StreamingSupported = firstSuccessStreaming
	}
	test.Success = anySuccess

	switch {
	case test.Lifecycle == CapabilityLifecycleCancelled || allCancelled:
		test.Lifecycle = CapabilityLifecycleCancelled
		test.Outcome = CapabilityOutcomeCancelled
	case anyActive:
		test.Lifecycle = CapabilityLifecycleActive
		if anySuccess {
			test.Outcome = CapabilityOutcomePartial
		} else {
			test.Outcome = CapabilityOutcomeUnknown
		}
	case !allTerminal:
		test.Lifecycle = CapabilityLifecyclePending
		test.Outcome = CapabilityOutcomeUnknown
	case anySuccess && (anyFailed || anyCancelled):
		test.Lifecycle = CapabilityLifecycleDone
		test.Outcome = CapabilityOutcomePartial
		test.Error = nil
	case anySuccess:
		test.Lifecycle = CapabilityLifecycleDone
		test.Outcome = CapabilityOutcomeSuccess
		test.Error = nil
	case anyFailed:
		test.Lifecycle = CapabilityLifecycleDone
		test.Outcome = CapabilityOutcomeFailed
	case anyCancelled:
		test.Lifecycle = CapabilityLifecycleCancelled
		test.Outcome = CapabilityOutcomeCancelled
	default:
		test.Lifecycle = CapabilityLifecyclePending
		test.Outcome = CapabilityOutcomeUnknown
	}
	test.Status = deriveCapabilityProtocolStatus(test.Lifecycle, test.Outcome)
}

func capabilityProtocolResultsFromResponse(resp CapabilityTestResponse) []CapabilityProtocolJobResult {
	results := make([]CapabilityProtocolJobResult, 0, len(resp.Tests))
	for _, test := range resp.Tests {
		status := CapabilityProtocolStatusFailed
		if test.Success {
			status = CapabilityProtocolStatusCompleted
		}
		modelResults := make([]CapabilityModelJobResult, 0, len(test.ModelResults))
		for _, modelResult := range test.ModelResults {
			modelStatus := CapabilityModelStatusFailed
			if modelResult.Success {
				modelStatus = CapabilityModelStatusSuccess
			} else if modelResult.Skipped {
				modelStatus = CapabilityModelStatusSkipped
			}
			modelResults = append(modelResults, CapabilityModelJobResult{
				Model:              modelResult.Model,
				ActualModel:        modelResult.ActualModel,
				Status:             modelStatus,
				Lifecycle:          capabilityModelLifecycleFromLegacy(modelStatus),
				Outcome:            capabilityModelOutcomeFromLegacy(modelStatus, modelResult.Success),
				Success:            modelResult.Success,
				Latency:            modelResult.Latency,
				StreamingSupported: modelResult.StreamingSupported,
				Error:              modelResult.Error,
				Reason:             modelResult.Error,
				StartedAt:          modelResult.StartedAt,
				TestedAt:           modelResult.TestedAt,
			})
		}
		results = append(results, CapabilityProtocolJobResult{
			Protocol:           test.Protocol,
			Status:             status,
			Lifecycle:          capabilityProtocolLifecycleFromLegacy(status),
			Outcome:            capabilityProtocolOutcomeFromLegacy(status, test.Success, test.SuccessCount, test.AttemptedModels),
			Reason:             test.Error,
			Success:            test.Success,
			Latency:            test.Latency,
			StreamingSupported: test.StreamingSupported,
			TestedModel:        test.TestedModel,
			ModelResults:       modelResults,
			SuccessCount:       test.SuccessCount,
			AttemptedModels:    test.AttemptedModels,
			Error:              test.Error,
			TestedAt:           test.TestedAt,
		})
	}
	return results
}

func createCapabilityJobFromResponse(channelID int, channelName, channelKind, sourceType string, protocols []string, timeout time.Duration, effectiveRPM int, resp CapabilityTestResponse, cacheHit bool) *CapabilityTestJob {
	now := time.Now().Format(time.RFC3339Nano)
	job := &CapabilityTestJob{
		JobID:               newCapabilityJobID(),
		IdentityKey:         "",
		ExecutionKey:        "",
		ChannelID:           channelID,
		ChannelName:         channelName,
		ChannelKind:         channelKind,
		SourceType:          sourceType,
		Status:              CapabilityJobStatusCompleted,
		Lifecycle:           CapabilityLifecycleDone,
		Outcome:             CapabilityOutcomeSuccess,
		Tests:               capabilityProtocolResultsFromResponse(resp),
		RedirectTests:       append([]RedirectModelResult(nil), resp.RedirectTests...),
		CompatibleProtocols: append([]string(nil), resp.CompatibleProtocols...),
		TotalDuration:       resp.TotalDuration,
		StartedAt:           now,
		UpdatedAt:           now,
		FinishedAt:          now,
		CacheHit:            cacheHit,
		RunMode:             CapabilityRunModeCacheHit,
		TargetProtocols:     append([]string(nil), protocols...),
		TimeoutMilliseconds: int(timeout / time.Millisecond),
		EffectiveRPM:        effectiveRPM,
	}
	recomputeCapabilityJob(job)
	return job
}

func getCapabilityTestChannel(cfgManager *config.ConfigManager, channelKind string, id int) (*config.UpstreamConfig, error) {
	cfg := cfgManager.GetConfig()
	var channels []config.UpstreamConfig
	switch channelKind {
	case "messages":
		channels = cfg.Upstream
	case "responses":
		channels = cfg.ResponsesUpstream
	case "gemini":
		channels = cfg.GeminiUpstream
	case "chat":
		channels = cfg.ChatUpstream
	case "images":
		channels = cfg.ImagesUpstream
	default:
		return nil, fmt.Errorf("invalid channel kind")
	}

	if id < 0 || id >= len(channels) {
		return nil, fmt.Errorf("channel not found")
	}

	channel := channels[id]
	return &channel, nil
}

func GetCapabilityTestJobStatus(cfgManager *config.ConfigManager, channelKind string) gin.HandlerFunc {
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

		channel, getErr := getCapabilityTestChannel(cfgManager, channelKind, id)
		if getErr != nil {
			if !capabilityJobMatchesChannel(job, nil, channelKind, id) {
				c.JSON(http.StatusNotFound, gin.H{"error": "Capability test job not found"})
				return
			}
			c.JSON(http.StatusOK, job)
			return
		}

		if !capabilityJobMatchesChannel(job, channel, channelKind, id) {
			c.JSON(http.StatusNotFound, gin.H{"error": "Capability test job not found"})
			return
		}

		job.ChannelID = id
		job.ChannelName = channel.Name
		job.SourceType = channel.ServiceType

		c.JSON(http.StatusOK, job)
	}
}

func capabilityModelLifecycleFromLegacy(status CapabilityModelStatus) CapabilityLifecycle {
	switch status {
	case CapabilityModelStatusQueued:
		return CapabilityLifecyclePending
	case CapabilityModelStatusRunning:
		return CapabilityLifecycleActive
	case CapabilityModelStatusSkipped:
		return CapabilityLifecycleDone
	default:
		return CapabilityLifecycleDone
	}
}

func capabilityModelOutcomeFromLegacy(status CapabilityModelStatus, success bool) CapabilityOutcome {
	switch {
	case success || status == CapabilityModelStatusSuccess:
		return CapabilityOutcomeSuccess
	case status == CapabilityModelStatusFailed:
		return CapabilityOutcomeFailed
	case status == CapabilityModelStatusSkipped:
		return CapabilityOutcomeUnknown
	case status == CapabilityModelStatusRunning || status == CapabilityModelStatusQueued:
		return CapabilityOutcomeUnknown
	default:
		return CapabilityOutcomeUnknown
	}
}

func capabilityProtocolLifecycleFromLegacy(status CapabilityProtocolStatus) CapabilityLifecycle {
	switch status {
	case CapabilityProtocolStatusQueued:
		return CapabilityLifecyclePending
	case CapabilityProtocolStatusRunning:
		return CapabilityLifecycleActive
	default:
		return CapabilityLifecycleDone
	}
}

func capabilityProtocolOutcomeFromLegacy(status CapabilityProtocolStatus, success bool, successCount, attemptedModels int) CapabilityOutcome {
	if status == CapabilityProtocolStatusRunning || status == CapabilityProtocolStatusQueued {
		return CapabilityOutcomeUnknown
	}
	if success {
		if attemptedModels > 0 && successCount > 0 && successCount < attemptedModels {
			return CapabilityOutcomePartial
		}
		return CapabilityOutcomeSuccess
	}
	return CapabilityOutcomeFailed
}

func getCapabilityDisplayMode(job *CapabilityTestJob) CapabilityRunMode {
	if job == nil {
		return CapabilityRunModeFresh
	}
	if job.CacheHit {
		return CapabilityRunModeCacheHit
	}
	if job.IsResumed && job.RunMode != "" {
		return job.RunMode
	}
	if job.RunMode != "" {
		return job.RunMode
	}
	return CapabilityRunModeFresh
}

func deriveCapabilityJobStatus(lifecycle CapabilityLifecycle, outcome CapabilityOutcome) CapabilityJobStatus {
	switch lifecycle {
	case CapabilityLifecyclePending:
		return CapabilityJobStatusQueued
	case CapabilityLifecycleActive:
		return CapabilityJobStatusRunning
	case CapabilityLifecycleCancelled:
		return CapabilityJobStatusCancelled
	case CapabilityLifecycleDone:
		switch outcome {
		case CapabilityOutcomeSuccess, CapabilityOutcomePartial:
			return CapabilityJobStatusCompleted
		case CapabilityOutcomeFailed, CapabilityOutcomeUnknown:
			return CapabilityJobStatusFailed
		case CapabilityOutcomeCancelled:
			return CapabilityJobStatusCancelled
		}
	}
	return CapabilityJobStatusFailed
}

func deriveCapabilityProtocolStatus(lifecycle CapabilityLifecycle, outcome CapabilityOutcome) CapabilityProtocolStatus {
	switch lifecycle {
	case CapabilityLifecyclePending:
		return CapabilityProtocolStatusQueued
	case CapabilityLifecycleActive:
		return CapabilityProtocolStatusRunning
	case CapabilityLifecycleCancelled:
		return CapabilityProtocolStatusFailed
	case CapabilityLifecycleDone:
		switch outcome {
		case CapabilityOutcomeSuccess, CapabilityOutcomePartial:
			return CapabilityProtocolStatusCompleted
		default:
			return CapabilityProtocolStatusFailed
		}
	}
	return CapabilityProtocolStatusFailed
}

func deriveCapabilityModelStatus(lifecycle CapabilityLifecycle, outcome CapabilityOutcome) CapabilityModelStatus {
	switch lifecycle {
	case CapabilityLifecyclePending:
		return CapabilityModelStatusQueued
	case CapabilityLifecycleActive:
		return CapabilityModelStatusRunning
	case CapabilityLifecycleCancelled:
		return CapabilityModelStatusSkipped
	case CapabilityLifecycleDone:
		switch outcome {
		case CapabilityOutcomeSuccess:
			return CapabilityModelStatusSuccess
		case CapabilityOutcomeFailed:
			return CapabilityModelStatusFailed
		default:
			return CapabilityModelStatusSkipped
		}
	}
	return CapabilityModelStatusFailed
}

func parseCapabilityChannelID(c *gin.Context) (int, error) {
	return strconv.Atoi(c.Param("id"))
}

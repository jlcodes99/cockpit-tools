package handlers

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
)

// TestRunRedirectVerification_UsesChannelServiceTypeForVirtualProtocol 确保跨协议测试按上游真实类型发请求
func TestExpandCapabilityProtocolsForChannel_IncludesSameProtocolWhenModelMappingExists(t *testing.T) {
	channel := &config.UpstreamConfig{
		ServiceType: "claude",
		ModelMapping: map[string]string{
			"claude-sonnet-4-6": "glm-5.1-pro",
		},
	}

	got := expandCapabilityProtocolsForChannel("messages", channel, []string{"messages"})
	want := []string{"messages->messages", "messages"}
	if len(got) != len(want) {
		t.Fatalf("protocols length=%d, want %d: %v", len(got), len(want), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("protocols[%d]=%q, want %q: %v", i, got[i], want[i], got)
		}
	}
}

func TestRunRedirectVerification_UsesChannelServiceTypeForVirtualProtocol(t *testing.T) {
	resetCapabilityTestState()

	var requestCount atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requestCount.Add(1)
		if r.URL.Path != "/v1/responses" {
			http.Error(w, "unexpected path: "+r.URL.Path, http.StatusBadRequest)
			return
		}
		if r.Header.Get("Originator") != "codex_cli_rs" {
			http.Error(w, "unexpected originator: "+r.Header.Get("Originator"), http.StatusBadRequest)
			return
		}
		body, err := io.ReadAll(r.Body)
		if err != nil {
			http.Error(w, "read body failed", http.StatusBadRequest)
			return
		}
		var payload map[string]interface{}
		if err := json.Unmarshal(body, &payload); err != nil {
			http.Error(w, "invalid json body", http.StatusBadRequest)
			return
		}
		if _, ok := payload["input"]; !ok {
			http.Error(w, "missing responses input field: "+string(body), http.StatusBadRequest)
			return
		}
		if _, ok := payload["messages"]; ok {
			http.Error(w, "unexpected messages field: "+string(body), http.StatusBadRequest)
			return
		}

		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n"))
		_, _ = w.Write([]byte("event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n"))
	}))
	defer server.Close()

	channel := &config.UpstreamConfig{
		Name:        "responses-channel",
		ServiceType: "responses",
		BaseURL:     server.URL,
		APIKeys:     []string{"test-key"},
		ModelMapping: map[string]string{
			"claude-opus-4-7": "gpt-5.5",
		},
	}
	job := newCapabilityTestJob(1, channel.Name, "messages", channel.ServiceType, []string{"messages->responses"}, 5*time.Second, 600)
	capabilityJobs.create(job)

	results := runRedirectVerification(context.Background(), channel, "messages", "messages", 5*time.Second, 600, job.JobID, nil, 1, "test-key", "test-dispatcher", nil, []string{"claude-opus-4-7"})
	if len(results) != 1 {
		t.Fatalf("results length=%d, want 1", len(results))
	}
	if !results[0].Success {
		t.Fatalf("redirect result success=false, error=%v", results[0].Error)
	}
	if got := requestCount.Load(); got != 1 {
		t.Fatalf("request count=%d, want 1", got)
	}
}

func TestRunRedirectVerification_SharedActualModel(t *testing.T) {
	// 准备测试数据
	channel := &config.UpstreamConfig{
		Name:        "test-channel",
		ServiceType: "claude",
		ModelMapping: map[string]string{
			"claude-sonnet-4-6":          "glm-5.1-pro",
			"claude-sonnet-4-5-20250929": "glm-5.1-pro", // 重定向到同一个模型
			"claude-haiku-4-5-20251001":  "glm-5.1",
		},
	}

	// 创建测试 job
	jobID := newCapabilityJobID()
	job := newCapabilityTestJob(1, "test-channel", "messages", "claude", []string{"messages"}, 10*time.Second, 10)
	job.JobID = jobID
	capabilityJobs.create(job)

	// 模拟重定向验证（不实际发送请求）
	sourceTab := "messages"

	// 获取探测模型
	probeModels, err := getCapabilityProbeModels(sourceTab)
	if err != nil {
		t.Fatalf("getCapabilityProbeModels failed: %v", err)
	}

	// 构建重定向模型列表（去重 actualModel）
	testedActualModels := make(map[string]bool)
	var redirectedModels []RedirectModelResult
	for _, m := range probeModels {
		actual := config.RedirectModel(m, channel)
		if actual != m && !testedActualModels[actual] {
			redirectedModels = append(redirectedModels, RedirectModelResult{
				ProbeModel:  m,
				ActualModel: actual,
			})
			testedActualModels[actual] = true
		}
	}

	// 验证去重逻辑
	if len(redirectedModels) != 2 {
		t.Fatalf("expected 2 unique actualModels, got %d", len(redirectedModels))
	}

	// 初始化虚拟协议占位符
	channelServiceType := serviceTypeToChannelKind(channel.ServiceType)
	virtualProtocol := sourceTab + "->" + channelServiceType

	capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
		modelResults := make([]CapabilityModelJobResult, 0)
		for _, probeModel := range probeModels {
			actualModel := config.RedirectModel(probeModel, channel)
			if actualModel != probeModel {
				modelResults = append(modelResults, CapabilityModelJobResult{
					Model:       probeModel,
					ActualModel: actualModel,
					Status:      CapabilityModelStatusQueued,
					Lifecycle:   CapabilityLifecyclePending,
					Outcome:     CapabilityOutcomeUnknown,
				})
			}
		}
		job.Tests = append([]CapabilityProtocolJobResult{{
			Protocol:        virtualProtocol,
			Status:          CapabilityProtocolStatusQueued,
			Lifecycle:       CapabilityLifecyclePending,
			Outcome:         CapabilityOutcomeUnknown,
			AttemptedModels: len(modelResults),
			ModelResults:    modelResults,
			TestedAt:        time.Now().Format(time.RFC3339Nano),
		}}, job.Tests...)
	})

	// 验证初始状态
	updatedJob, _ := capabilityJobs.get(jobID)
	if len(updatedJob.Tests) == 0 {
		t.Fatal("expected virtual protocol test to be created")
	}
	virtualTest := updatedJob.Tests[0]
	if virtualTest.Protocol != virtualProtocol {
		t.Fatalf("expected protocol %s, got %s", virtualProtocol, virtualTest.Protocol)
	}
	if len(virtualTest.ModelResults) != 3 {
		t.Fatalf("expected 3 probe models, got %d", len(virtualTest.ModelResults))
	}

	// 模拟测试结果（只测试 2 个不同的 actualModel）
	results := []RedirectModelResult{
		{
			ProbeModel:         "claude-sonnet-4-6",
			ActualModel:        "glm-5.1-pro",
			Success:            true,
			Latency:            2498,
			StreamingSupported: true,
			StartedAt:          time.Now().Format(time.RFC3339Nano),
			TestedAt:           time.Now().Format(time.RFC3339Nano),
		},
		{
			ProbeModel:         "claude-haiku-4-5-20251001",
			ActualModel:        "glm-5.1",
			Success:            true,
			Latency:            1515,
			StreamingSupported: true,
			StartedAt:          time.Now().Format(time.RFC3339Nano),
			TestedAt:           time.Now().Format(time.RFC3339Nano),
		},
	}

	// 更新最终状态（模拟 runRedirectVerification 的结尾逻辑）
	successCount := countRedirectSuccesses(results)
	actualModelResults := make(map[string]RedirectModelResult)
	for _, r := range results {
		actualModelResults[r.ActualModel] = r
	}

	capabilityJobs.update(jobID, func(job *CapabilityTestJob) {
		for i := range job.Tests {
			if job.Tests[i].Protocol == virtualProtocol {
				// 更新所有模型的测试结果
				for j := range job.Tests[i].ModelResults {
					actualModel := job.Tests[i].ModelResults[j].ActualModel
					if result, ok := actualModelResults[actualModel]; ok {
						modelStatus := CapabilityModelStatusFailed
						if result.Success {
							modelStatus = CapabilityModelStatusSuccess
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

	// 验证最终状态
	finalJob, _ := capabilityJobs.get(jobID)
	finalTest := finalJob.Tests[0]

	// 验证协议状态
	if finalTest.Lifecycle != CapabilityLifecycleDone {
		t.Fatalf("expected lifecycle done, got %s", finalTest.Lifecycle)
	}
	if finalTest.Outcome != CapabilityOutcomeSuccess {
		t.Fatalf("expected outcome success, got %s", finalTest.Outcome)
	}
	if !finalTest.Success {
		t.Fatal("expected success to be true")
	}
	if finalTest.SuccessCount != 3 {
		t.Fatalf("expected successCount 3 (all probe models), got %d", finalTest.SuccessCount)
	}

	// 验证所有探测模型都有测试结果
	for _, modelResult := range finalTest.ModelResults {
		if modelResult.Status == CapabilityModelStatusQueued {
			t.Fatalf("model %s still in queued state", modelResult.Model)
		}
		if modelResult.Lifecycle != CapabilityLifecycleDone {
			t.Fatalf("model %s lifecycle = %s, want done", modelResult.Model, modelResult.Lifecycle)
		}
		if modelResult.Outcome != CapabilityOutcomeSuccess {
			t.Fatalf("model %s outcome = %s, want success", modelResult.Model, modelResult.Outcome)
		}
		if !modelResult.Success {
			t.Fatalf("model %s success = false, want true", modelResult.Model)
		}
	}

	// 验证共享 actualModel 的模型有相同的测试结果
	var glmProResults []CapabilityModelJobResult
	for _, mr := range finalTest.ModelResults {
		if mr.ActualModel == "glm-5.1-pro" {
			glmProResults = append(glmProResults, mr)
		}
	}
	if len(glmProResults) != 2 {
		t.Fatalf("expected 2 models redirected to glm-5.1-pro, got %d", len(glmProResults))
	}
	// 验证它们的测试结果相同（除了 Model 字段）
	if glmProResults[0].Latency != glmProResults[1].Latency {
		t.Fatalf("shared actualModel should have same latency")
	}
	if glmProResults[0].StreamingSupported != glmProResults[1].StreamingSupported {
		t.Fatalf("shared actualModel should have same streamingSupported")
	}
}

func TestFilterCapabilityProbeModels(t *testing.T) {
	probeModels := []string{"claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-4-6"}
	filtered := filterCapabilityProbeModels(probeModels, []string{"claude-opus-4-7"})

	if len(filtered) != 1 {
		t.Fatalf("filtered len = %d, want 1: %#v", len(filtered), filtered)
	}
	if filtered[0] != "claude-opus-4-7" {
		t.Fatalf("filtered[0] = %q, want claude-opus-4-7", filtered[0])
	}

	unfiltered := filterCapabilityProbeModels(probeModels, nil)
	if len(unfiltered) != len(probeModels) {
		t.Fatalf("unfiltered len = %d, want %d", len(unfiltered), len(probeModels))
	}
}

func TestUpdateCapabilityJobModelResultsByActualModel_UpdatesSharedModelsImmediately(t *testing.T) {
	now := time.Now().Format(time.RFC3339Nano)
	job := &CapabilityTestJob{
		JobID:       "cap-test-shared-actual",
		ChannelID:   1,
		ChannelName: "test-channel",
		ChannelKind: "messages",
		SourceType:  "openai",
		Tests: []CapabilityProtocolJobResult{
			{
				Protocol:  "messages->chat",
				Status:    CapabilityProtocolStatusQueued,
				Lifecycle: CapabilityLifecyclePending,
				Outcome:   CapabilityOutcomeUnknown,
				ModelResults: []CapabilityModelJobResult{
					{Model: "claude-opus-4-7", ActualModel: "gpt-5.5", Status: CapabilityModelStatusQueued, Lifecycle: CapabilityLifecyclePending, Outcome: CapabilityOutcomeUnknown},
					{Model: "claude-opus-4-6", ActualModel: "gpt-5.5", Status: CapabilityModelStatusQueued, Lifecycle: CapabilityLifecyclePending, Outcome: CapabilityOutcomeUnknown},
					{Model: "claude-sonnet-4-6", ActualModel: "gpt-5.4", Status: CapabilityModelStatusQueued, Lifecycle: CapabilityLifecyclePending, Outcome: CapabilityOutcomeUnknown},
				},
				TestedAt: now,
			},
		},
		UpdatedAt: now,
	}

	updated := updateCapabilityJobModelResultsByActualModel(job, "messages->chat", "gpt-5.5", CapabilityModelStatusRunning, ModelTestResult{
		ActualModel: "gpt-5.5",
		StartedAt:   now,
	})
	if updated != 2 {
		t.Fatalf("updated running models=%d, want 2", updated)
	}

	for _, mr := range job.Tests[0].ModelResults[:2] {
		if mr.Status != CapabilityModelStatusRunning {
			t.Fatalf("model %s status=%s, want running", mr.Model, mr.Status)
		}
		if mr.Lifecycle != CapabilityLifecycleActive {
			t.Fatalf("model %s lifecycle=%s, want active", mr.Model, mr.Lifecycle)
		}
	}
	if job.Tests[0].ModelResults[2].Status != CapabilityModelStatusQueued {
		t.Fatalf("unrelated model status=%s, want queued", job.Tests[0].ModelResults[2].Status)
	}

	updated = updateCapabilityJobModelResultsByActualModel(job, "messages->chat", "gpt-5.5", CapabilityModelStatusSuccess, ModelTestResult{
		ActualModel:        "gpt-5.5",
		Success:            true,
		Latency:            1234,
		StreamingSupported: true,
		StartedAt:          now,
		TestedAt:           now,
	})
	if updated != 2 {
		t.Fatalf("updated success models=%d, want 2", updated)
	}

	for _, mr := range job.Tests[0].ModelResults[:2] {
		if mr.Status != CapabilityModelStatusSuccess {
			t.Fatalf("model %s status=%s, want success", mr.Model, mr.Status)
		}
		if mr.Outcome != CapabilityOutcomeSuccess {
			t.Fatalf("model %s outcome=%s, want success", mr.Model, mr.Outcome)
		}
		if mr.Latency != 1234 {
			t.Fatalf("model %s latency=%d, want 1234", mr.Model, mr.Latency)
		}
	}
}

// TestVirtualProtocolSnapshot_InitialState 测试虚拟协议快照的初始状态
func TestVirtualProtocolSnapshot_InitialState(t *testing.T) {
	channel := &config.UpstreamConfig{
		Name:        "test-channel",
		ServiceType: "openai",
		ModelMapping: map[string]string{
			"claude-sonnet-4-6":         "gpt-5.5",
			"claude-haiku-4-5-20251001": "gpt-5.3-codex",
		},
		APIKeys: []string{"test-key"},
	}

	// 模拟 GetCapabilitySnapshot 的逻辑
	sourceTab := "messages"
	channelServiceType := serviceTypeToChannelKind(channel.ServiceType)
	virtualProtocol := sourceTab + "->" + channelServiceType

	// 获取探测模型
	probeModels, err := getCapabilityProbeModels(sourceTab)
	if err != nil {
		t.Fatalf("getCapabilityProbeModels failed: %v", err)
	}

	// 检查是否有模型被重定向
	hasRedirect := false
	for _, m := range probeModels {
		actual := config.RedirectModel(m, channel)
		if actual != m {
			hasRedirect = true
			break
		}
	}

	if !hasRedirect {
		t.Fatal("expected at least one model to be redirected")
	}

	// 构建模型结果列表
	modelResults := make([]CapabilityModelJobResult, 0)
	for _, m := range probeModels {
		actual := config.RedirectModel(m, channel)
		if actual == m {
			continue
		}
		modelResults = append(modelResults, CapabilityModelJobResult{
			Model:       m,
			ActualModel: actual,
			Status:      "idle",
		})
	}

	// 验证初始状态
	if len(modelResults) == 0 {
		t.Fatal("expected at least one model in results")
	}

	for _, mr := range modelResults {
		if mr.Status != "idle" {
			t.Fatalf("expected status idle, got %s", mr.Status)
		}
		if mr.Model == "" {
			t.Fatal("expected model to be set")
		}
		if mr.ActualModel == "" {
			t.Fatal("expected actualModel to be set")
		}
		if mr.Model == mr.ActualModel {
			t.Fatalf("model %s should be redirected", mr.Model)
		}
	}

	// 验证虚拟协议名称
	expectedProtocol := "messages->chat"
	if virtualProtocol != expectedProtocol {
		t.Fatalf("expected protocol %s, got %s", expectedProtocol, virtualProtocol)
	}
}

// TestCountRedirectSuccesses 测试成功计数函数
func TestCountRedirectSuccesses(t *testing.T) {
	tests := []struct {
		name     string
		results  []RedirectModelResult
		expected int
	}{
		{
			name:     "empty results",
			results:  []RedirectModelResult{},
			expected: 0,
		},
		{
			name: "all success",
			results: []RedirectModelResult{
				{Success: true},
				{Success: true},
			},
			expected: 2,
		},
		{
			name: "all failed",
			results: []RedirectModelResult{
				{Success: false},
				{Success: false},
			},
			expected: 0,
		},
		{
			name: "mixed",
			results: []RedirectModelResult{
				{Success: true},
				{Success: false},
				{Success: true},
			},
			expected: 2,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := countRedirectSuccesses(tt.results)
			if got != tt.expected {
				t.Fatalf("expected %d, got %d", tt.expected, got)
			}
		})
	}
}

// TestUpdateCapabilityJobModelResult_VirtualProtocol 测试虚拟协议模型结果更新
func TestUpdateCapabilityJobModelResult_VirtualProtocol(t *testing.T) {
	job := newCapabilityTestJob(1, "test-channel", "messages", "claude", []string{"messages"}, 10*time.Second, 10)

	// 添加虚拟协议测试
	virtualProtocol := "messages->chat"
	job.Tests = append([]CapabilityProtocolJobResult{{
		Protocol: virtualProtocol,
		Status:   CapabilityProtocolStatusQueued,
		ModelResults: []CapabilityModelJobResult{
			{
				Model:       "claude-sonnet-4-6",
				ActualModel: "gpt-5.5",
				Status:      CapabilityModelStatusQueued,
			},
		},
	}}, job.Tests...)

	// 更新为 running
	updateCapabilityJobModelResult(job, virtualProtocol, "claude-sonnet-4-6", CapabilityModelStatusRunning, ModelTestResult{
		Model:       "claude-sonnet-4-6",
		ActualModel: "gpt-5.5",
		StartedAt:   time.Now().Format(time.RFC3339Nano),
	})

	if job.Tests[0].ModelResults[0].Status != CapabilityModelStatusRunning {
		t.Fatalf("expected status running, got %s", job.Tests[0].ModelResults[0].Status)
	}
	if job.Tests[0].ModelResults[0].Lifecycle != CapabilityLifecycleActive {
		t.Fatalf("expected lifecycle active, got %s", job.Tests[0].ModelResults[0].Lifecycle)
	}

	// 更新为 success
	updateCapabilityJobModelResult(job, virtualProtocol, "claude-sonnet-4-6", CapabilityModelStatusSuccess, ModelTestResult{
		Model:              "claude-sonnet-4-6",
		ActualModel:        "gpt-5.5",
		Success:            true,
		Latency:            2000,
		StreamingSupported: true,
		StartedAt:          time.Now().Format(time.RFC3339Nano),
		TestedAt:           time.Now().Format(time.RFC3339Nano),
	})

	mr := job.Tests[0].ModelResults[0]
	if mr.Status != CapabilityModelStatusSuccess {
		t.Fatalf("expected status success, got %s", mr.Status)
	}
	if mr.Lifecycle != CapabilityLifecycleDone {
		t.Fatalf("expected lifecycle done, got %s", mr.Lifecycle)
	}
	if mr.Outcome != CapabilityOutcomeSuccess {
		t.Fatalf("expected outcome success, got %s", mr.Outcome)
	}
	if !mr.Success {
		t.Fatal("expected success to be true")
	}
	if mr.Latency != 2000 {
		t.Fatalf("expected latency 2000, got %d", mr.Latency)
	}
	if !mr.StreamingSupported {
		t.Fatal("expected streamingSupported to be true")
	}
}

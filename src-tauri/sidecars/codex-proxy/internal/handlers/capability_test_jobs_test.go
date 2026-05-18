package handlers

import (
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func TestNewCapabilityTestJobWithModels_UsesRequestedModelsOnly(t *testing.T) {
	job := newCapabilityTestJobWithModels(1, "channel", "messages", "claude", []string{"messages"}, 10*time.Second, 10, []string{"claude-opus-4-7"})

	if len(job.Tests) != 1 {
		t.Fatalf("tests len = %d, want 1", len(job.Tests))
	}
	modelResults := job.Tests[0].ModelResults
	if len(modelResults) != 1 {
		t.Fatalf("model results len = %d, want 1: %#v", len(modelResults), modelResults)
	}
	if modelResults[0].Model != "claude-opus-4-7" {
		t.Fatalf("model = %q, want claude-opus-4-7", modelResults[0].Model)
	}
	if modelResults[0].Status != CapabilityModelStatusQueued {
		t.Fatalf("status = %s, want queued", modelResults[0].Status)
	}
	if job.Tests[0].AttemptedModels != 1 {
		t.Fatalf("attempted models = %d, want 1", job.Tests[0].AttemptedModels)
	}
}

func TestNewCapabilityTestJobWithModels_FallsBackToProbeModels(t *testing.T) {
	job := newCapabilityTestJobWithModels(1, "channel", "messages", "claude", []string{"messages"}, 10*time.Second, 10, nil)

	if len(job.Tests) != 1 {
		t.Fatalf("tests len = %d, want 1", len(job.Tests))
	}
	if len(job.Tests[0].ModelResults) <= 1 {
		t.Fatalf("expected full probe model placeholders, got %#v", job.Tests[0].ModelResults)
	}
}

func TestCapabilityJobStore_GetOrCreateByLookupKey_Concurrent(t *testing.T) {
	store := &capabilityTestJobStore{
		jobs:      make(map[string]*CapabilityTestJob),
		lookupKey: make(map[string]string),
	}

	var buildCount int32
	var reusedCount int32
	const total = 20
	lookupKey := "capability:messages:1"

	jobIDs := make(chan string, total)
	var wg sync.WaitGroup
	wg.Add(total)

	for i := 0; i < total; i++ {
		go func() {
			defer wg.Done()
			job, reused := store.getOrCreateByLookupKey(lookupKey, func() *CapabilityTestJob {
				atomic.AddInt32(&buildCount, 1)
				return newCapabilityTestJob(1, "channel", "messages", "claude", []string{"messages"}, 10*time.Second, 10)
			})
			if reused {
				atomic.AddInt32(&reusedCount, 1)
			}
			jobIDs <- job.JobID
		}()
	}

	wg.Wait()
	close(jobIDs)

	var firstID string
	for id := range jobIDs {
		if firstID == "" {
			firstID = id
			continue
		}
		if id != firstID {
			t.Fatalf("jobID mismatch: got %s, want %s", id, firstID)
		}
	}

	if buildCount != 1 {
		t.Fatalf("builder called %d times, want 1", buildCount)
	}
	if reusedCount != total-1 {
		t.Fatalf("reusedCount = %d, want %d", reusedCount, total-1)
	}
}

func TestRecomputeCapabilityJob_PartialSuccess(t *testing.T) {
	job := newCapabilityTestJob(1, "channel", "messages", "claude", []string{"messages"}, 10*time.Second, 10)
	job.Tests[0].ModelResults = []CapabilityModelJobResult{
		{Model: "a", Status: CapabilityModelStatusSuccess, Lifecycle: CapabilityLifecycleDone, Outcome: CapabilityOutcomeSuccess, Success: true},
		{Model: "b", Status: CapabilityModelStatusFailed, Lifecycle: CapabilityLifecycleDone, Outcome: CapabilityOutcomeFailed, Success: false},
	}
	job.Tests[0].AttemptedModels = 2

	recomputeCapabilityJob(job)

	if job.Outcome != CapabilityOutcomePartial {
		t.Fatalf("job outcome = %s, want partial", job.Outcome)
	}
	if job.Lifecycle != CapabilityLifecycleDone {
		t.Fatalf("job lifecycle = %s, want done", job.Lifecycle)
	}
	if job.Tests[0].Outcome != CapabilityOutcomePartial {
		t.Fatalf("protocol outcome = %s, want partial", job.Tests[0].Outcome)
	}
}

func TestUpdateCapabilityJobModelResult_SetsCancelledReason(t *testing.T) {
	job := newCapabilityTestJob(1, "channel", "messages", "claude", []string{"messages"}, 10*time.Second, 10)
	job.Tests[0].ModelResults = []CapabilityModelJobResult{{Model: "a", Status: CapabilityModelStatusRunning, Lifecycle: CapabilityLifecycleActive, Outcome: CapabilityOutcomeUnknown}}

	reason := "cancelled"
	updateCapabilityJobModelResult(job, "messages", "a", CapabilityModelStatusSkipped, ModelTestResult{Model: "a", Error: &reason})

	got := job.Tests[0].ModelResults[0]
	if got.Lifecycle != CapabilityLifecycleCancelled {
		t.Fatalf("lifecycle = %s, want cancelled", got.Lifecycle)
	}
	if got.Outcome != CapabilityOutcomeCancelled {
		t.Fatalf("outcome = %s, want cancelled", got.Outcome)
	}
}

func TestBuildCapabilityExecutionLookupKey_NormalizesProtocolsAndModels(t *testing.T) {
	keyA := buildCapabilityExecutionLookupKey("identity-a", "messages", []string{"responses", "messages"}, []string{"model-b", "model-a"}, "")
	keyB := buildCapabilityExecutionLookupKey("identity-a", "messages", []string{"messages", "responses"}, []string{"model-a", "model-b", "model-a"}, "")

	if keyA != keyB {
		t.Fatalf("execution lookup key mismatch: got %q want %q", keyA, keyB)
	}
}

func TestCancelCapabilityStateShape(t *testing.T) {
	job := newCapabilityTestJob(1, "channel", "messages", "claude", []string{"messages"}, 10*time.Second, 10)
	job.Status = CapabilityJobStatusCancelled
	job.Lifecycle = CapabilityLifecycleCancelled
	job.Outcome = CapabilityOutcomeCancelled
	job.Tests[0].Lifecycle = CapabilityLifecycleCancelled
	job.Tests[0].Outcome = CapabilityOutcomeCancelled
	job.Tests[0].ModelResults = []CapabilityModelJobResult{
		{Model: "queued", Status: CapabilityModelStatusSkipped, Lifecycle: CapabilityLifecycleDone, Outcome: CapabilityOutcomeUnknown},
		{Model: "running", Status: CapabilityModelStatusSkipped, Lifecycle: CapabilityLifecycleCancelled, Outcome: CapabilityOutcomeCancelled},
	}

	recomputeCapabilityJob(job)

	if job.Outcome != CapabilityOutcomeCancelled {
		t.Fatalf("job outcome = %s, want cancelled", job.Outcome)
	}
	if job.Tests[0].Outcome != CapabilityOutcomeCancelled {
		t.Fatalf("protocol outcome = %s, want cancelled", job.Tests[0].Outcome)
	}
}

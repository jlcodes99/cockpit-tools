package common

import (
	"strings"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
)

func TestRecordChannelLog_TruncatesAndMasks(t *testing.T) {
	store := metrics.NewChannelLogStore()
	longError := strings.Repeat("x", 260)

	RecordChannelLog(
		store,
		3,
		"model-a",
		"model-orig",
		502,
		123,
		false,
		"sk-test-very-secret",
		"https://example.com",
		longError,
		"Responses",
		true,
	)

	logs := store.Get(3)
	if len(logs) != 1 {
		t.Fatalf("logs count = %d, want 1", len(logs))
	}

	got := logs[0]
	if got.Model != "model-a" {
		t.Fatalf("model = %q, want model-a", got.Model)
	}
	if got.OriginalModel != "model-orig" {
		t.Fatalf("originalModel = %q, want model-orig", got.OriginalModel)
	}
	if got.StatusCode != 502 {
		t.Fatalf("statusCode = %d, want 502", got.StatusCode)
	}
	if got.DurationMs != 123 {
		t.Fatalf("durationMs = %d, want 123", got.DurationMs)
	}
	if got.Success {
		t.Fatalf("success = true, want false")
	}
	if got.BaseURL != "https://example.com" {
		t.Fatalf("baseURL = %q, want https://example.com", got.BaseURL)
	}
	if got.InterfaceType != "Responses" {
		t.Fatalf("interfaceType = %q, want Responses", got.InterfaceType)
	}
	if got.RequestSource != metrics.RequestSourceProxy {
		t.Fatalf("requestSource = %q, want %q", got.RequestSource, metrics.RequestSourceProxy)
	}
	if !got.IsRetry {
		t.Fatalf("isRetry = false, want true")
	}
	if len(got.ErrorInfo) != 200 {
		t.Fatalf("errorInfo len = %d, want 200", len(got.ErrorInfo))
	}
	if got.KeyMask == "sk-test-very-secret" || got.KeyMask == "" {
		t.Fatalf("keyMask = %q, want masked non-empty value", got.KeyMask)
	}
}

func TestRecordChannelLogWithSource_UsesExplicitSource(t *testing.T) {
	store := metrics.NewChannelLogStore()

	RecordChannelLogWithSource(
		store,
		1,
		"model-b",
		"",
		200,
		45,
		true,
		"sk-test-another-secret",
		"https://example.com",
		"",
		"Messages",
		false,
		metrics.RequestSourceCapabilityTest,
	)

	logs := store.Get(1)
	if len(logs) != 1 {
		t.Fatalf("logs count = %d, want 1", len(logs))
	}
	if logs[0].RequestSource != metrics.RequestSourceCapabilityTest {
		t.Fatalf("requestSource = %q, want %q", logs[0].RequestSource, metrics.RequestSourceCapabilityTest)
	}
}

func TestCompleteLog_MapsClientCanceledToCancelledStatus(t *testing.T) {
	store := metrics.NewChannelLogStore()
	requestID := CreatePendingLog(store, 0, "model-a", "", "sk-test-secret", "https://example.com", "Responses", "edits", metrics.RequestSourceProxy)

	CompleteLog(store, 0, requestID, 200, false, "client canceled", false)

	logs := store.Get(0)
	if len(logs) != 1 {
		t.Fatalf("logs count = %d, want 1", len(logs))
	}
	if logs[0].Operation != "edits" {
		t.Fatalf("operation = %q, want edits", logs[0].Operation)
	}
	if logs[0].Status != metrics.StatusCancelled {
		t.Fatalf("status = %q, want %q", logs[0].Status, metrics.StatusCancelled)
	}
	if logs[0].Success {
		t.Fatalf("success = true, want false")
	}
}

func TestCompleteLog_LeavesRealFailuresAsFailed(t *testing.T) {
	store := metrics.NewChannelLogStore()
	requestID := CreatePendingLog(store, 0, "model-a", "", "sk-test-secret", "https://example.com", "Responses", "", metrics.RequestSourceProxy)

	CompleteLog(store, 0, requestID, 502, false, "upstream timeout", false)

	logs := store.Get(0)
	if len(logs) != 1 {
		t.Fatalf("logs count = %d, want 1", len(logs))
	}
	if logs[0].Status != metrics.StatusFailed {
		t.Fatalf("status = %q, want %q", logs[0].Status, metrics.StatusFailed)
	}
}

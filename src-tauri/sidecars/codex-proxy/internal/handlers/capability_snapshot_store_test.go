package handlers

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

func TestCapabilitySnapshotStore_ReplaceFromJob(t *testing.T) {
	store := newCapabilitySnapshotStore()
	job := newCapabilityTestJob(1, "channel", "messages", "claude", []string{"messages"}, 0, 10)
	job.IdentityKey = "identity-a"
	job.Tests[0].Outcome = CapabilityOutcomeSuccess
	job.Tests[0].Lifecycle = CapabilityLifecycleDone
	job.CompatibleProtocols = []string{"messages"}
	job.Progress.TotalModels = 2
	job.Progress.CompletedModels = 2
	job.Lifecycle = CapabilityLifecycleDone
	job.Outcome = CapabilityOutcomeSuccess

	store.replaceFromJob(job.IdentityKey, job)

	snapshot, ok := store.get("identity-a")
	if !ok {
		t.Fatal("expected snapshot to exist")
	}
	if snapshot.IdentityKey != "identity-a" {
		t.Fatalf("identityKey=%s, want identity-a", snapshot.IdentityKey)
	}
	if snapshot.Lifecycle != CapabilityLifecycleDone {
		t.Fatalf("lifecycle=%s, want done", snapshot.Lifecycle)
	}
	if len(snapshot.CompatibleProtocols) != 1 || snapshot.CompatibleProtocols[0] != "messages" {
		t.Fatalf("compatibleProtocols=%v, want [messages]", snapshot.CompatibleProtocols)
	}
}

func TestCapabilitySnapshotStore_IsolatesDifferentIdentities(t *testing.T) {
	store := newCapabilitySnapshotStore()

	jobA := newCapabilityTestJob(1, "channel-a", "messages", "claude", []string{"messages"}, 0, 10)
	jobA.IdentityKey = "identity-a"
	jobA.Lifecycle = CapabilityLifecycleDone
	jobA.Outcome = CapabilityOutcomeSuccess
	jobA.Tests[0].Lifecycle = CapabilityLifecycleDone
	jobA.Tests[0].Outcome = CapabilityOutcomeSuccess
	store.replaceFromJob(jobA.IdentityKey, jobA)

	jobB := newCapabilityTestJob(2, "channel-b", "messages", "claude", []string{"responses"}, 0, 10)
	jobB.IdentityKey = "identity-b"
	jobB.Lifecycle = CapabilityLifecycleCancelled
	jobB.Outcome = CapabilityOutcomeCancelled
	jobB.Tests[0].Lifecycle = CapabilityLifecycleCancelled
	jobB.Tests[0].Outcome = CapabilityOutcomeCancelled
	store.replaceFromJob(jobB.IdentityKey, jobB)

	snapshotA, ok := store.get("identity-a")
	if !ok {
		t.Fatal("expected snapshotA to exist")
	}
	snapshotB, ok := store.get("identity-b")
	if !ok {
		t.Fatal("expected snapshotB to exist")
	}
	if snapshotA.IdentityKey == snapshotB.IdentityKey {
		t.Fatal("expected snapshots to be isolated by identity")
	}
	if snapshotA.Outcome != CapabilityOutcomeSuccess {
		t.Fatalf("snapshotA outcome=%s, want success", snapshotA.Outcome)
	}
	if snapshotB.Outcome != CapabilityOutcomeCancelled {
		t.Fatalf("snapshotB outcome=%s, want cancelled", snapshotB.Outcome)
	}
}

func TestCapabilitySnapshotStore_MergesMultipleJobsSameIdentity(t *testing.T) {
	store := newCapabilitySnapshotStore()
	const identityKey = "shared-identity"

	// Step 1: jobA (messages) → snapshot 含 messages
	jobA := newCapabilityTestJob(1, "ch", "messages", "claude", []string{"messages"}, 0, 10)
	jobA.IdentityKey = identityKey
	jobA.Lifecycle = CapabilityLifecycleActive
	jobA.Outcome = CapabilityOutcomeUnknown
	jobA.Tests[0].Lifecycle = CapabilityLifecycleActive
	jobA.Tests[0].Outcome = CapabilityOutcomeUnknown
	store.replaceFromJob(identityKey, jobA)

	snapshot, ok := store.get(identityKey)
	if !ok {
		t.Fatal("expected snapshot to exist after jobA")
	}
	if len(snapshot.ProtocolJobIDs) != 1 || snapshot.ProtocolJobIDs["messages"] != jobA.JobID {
		t.Fatalf("ProtocolJobIDs after jobA: %v, want {messages: %s}", snapshot.ProtocolJobIDs, jobA.JobID)
	}
	if len(snapshot.Tests) != 1 {
		t.Fatalf("Tests count after jobA: %d, want 1", len(snapshot.Tests))
	}
	if snapshot.Lifecycle != CapabilityLifecycleActive {
		t.Fatalf("lifecycle after jobA: %s, want active", snapshot.Lifecycle)
	}

	// Step 2: jobB (chat) → snapshot 含 messages + chat，messages 保持 active
	jobB := newCapabilityTestJob(1, "ch", "messages", "claude", []string{"chat"}, 0, 10)
	jobB.IdentityKey = identityKey
	jobB.Lifecycle = CapabilityLifecycleDone
	jobB.Outcome = CapabilityOutcomeSuccess
	jobB.Tests[0].Lifecycle = CapabilityLifecycleDone
	jobB.Tests[0].Outcome = CapabilityOutcomeSuccess
	store.replaceFromJob(identityKey, jobB)

	snapshot, ok = store.get(identityKey)
	if !ok {
		t.Fatal("expected snapshot to exist after jobB")
	}
	if len(snapshot.ProtocolJobIDs) != 2 {
		t.Fatalf("ProtocolJobIDs count after jobB: %d, want 2", len(snapshot.ProtocolJobIDs))
	}
	if snapshot.ProtocolJobIDs["messages"] != jobA.JobID {
		t.Fatalf("ProtocolJobIDs[messages] after jobB: %s, want %s", snapshot.ProtocolJobIDs["messages"], jobA.JobID)
	}
	if snapshot.ProtocolJobIDs["chat"] != jobB.JobID {
		t.Fatalf("ProtocolJobIDs[chat] after jobB: %s, want %s", snapshot.ProtocolJobIDs["chat"], jobB.JobID)
	}
	if len(snapshot.Tests) != 2 {
		t.Fatalf("Tests count after jobB: %d, want 2", len(snapshot.Tests))
	}
	// messages 应保持 active
	msgTest := findSnapshotTest(snapshot, "messages")
	if msgTest == nil {
		t.Fatal("messages test missing after jobB")
	}
	if msgTest.Lifecycle != CapabilityLifecycleActive {
		t.Fatalf("messages lifecycle after jobB: %s, want active", msgTest.Lifecycle)
	}
	chatTest := findSnapshotTest(snapshot, "chat")
	if chatTest == nil {
		t.Fatal("chat test missing after jobB")
	}
	if chatTest.Lifecycle != CapabilityLifecycleDone {
		t.Fatalf("chat lifecycle after jobB: %s, want done", chatTest.Lifecycle)
	}

	// Step 3: jobC (messages, done) → messages 更新为 done，chat 保持 done
	jobC := newCapabilityTestJob(1, "ch", "messages", "claude", []string{"messages"}, 0, 10)
	jobC.IdentityKey = identityKey
	jobC.Lifecycle = CapabilityLifecycleDone
	jobC.Outcome = CapabilityOutcomeSuccess
	jobC.Tests[0].Lifecycle = CapabilityLifecycleDone
	jobC.Tests[0].Outcome = CapabilityOutcomeSuccess
	store.replaceFromJob(identityKey, jobC)

	snapshot, ok = store.get(identityKey)
	if !ok {
		t.Fatal("expected snapshot to exist after jobC")
	}
	if len(snapshot.ProtocolJobIDs) != 2 {
		t.Fatalf("ProtocolJobIDs count after jobC: %d, want 2", len(snapshot.ProtocolJobIDs))
	}
	// messages 更新为新 jobId
	if snapshot.ProtocolJobIDs["messages"] != jobC.JobID {
		t.Fatalf("ProtocolJobIDs[messages] after jobC: %s, want %s", snapshot.ProtocolJobIDs["messages"], jobC.JobID)
	}
	// chat 保持 jobB.JobID
	if snapshot.ProtocolJobIDs["chat"] != jobB.JobID {
		t.Fatalf("ProtocolJobIDs[chat] after jobC: %s, want %s", snapshot.ProtocolJobIDs["chat"], jobB.JobID)
	}
	if snapshot.Lifecycle != CapabilityLifecycleDone {
		t.Fatalf("lifecycle after jobC: %s, want done (both protocols terminal)", snapshot.Lifecycle)
	}
	if snapshot.Outcome != CapabilityOutcomeSuccess {
		t.Fatalf("outcome after jobC: %s, want success", snapshot.Outcome)
	}
}

func TestGetCapabilitySnapshot_PreservesSameSourceRedirectProtocol(t *testing.T) {
	resetCapabilityTestState()
	gin.SetMode(gin.TestMode)

	cfg := config.Config{
		Upstream: []config.UpstreamConfig{
			{
				Name:        "claude-redirect-channel",
				BaseURL:     "https://example.test",
				APIKeys:     []string{"sk-test"},
				ServiceType: "claude",
				ModelMapping: map[string]string{
					"claude-sonnet-4-6": "claude-sonnet-4-6-upstream",
				},
			},
		},
	}
	data, err := json.Marshal(cfg)
	if err != nil {
		t.Fatalf("marshal config failed: %v", err)
	}

	configFile := filepath.Join(t.TempDir(), "config.json")
	if err := os.WriteFile(configFile, data, 0644); err != nil {
		t.Fatalf("write config failed: %v", err)
	}
	cfgManager, err := config.NewConfigManager(configFile)
	if err != nil {
		t.Fatalf("create config manager failed: %v", err)
	}
	t.Cleanup(func() { cfgManager.Close() })

	r := gin.New()
	r.GET("/messages/channels/:id/capability-snapshot", GetCapabilitySnapshot(cfgManager, "messages"))

	req := httptest.NewRequest(http.MethodGet, "/messages/channels/0/capability-snapshot?sourceTab=messages", nil)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", w.Code, w.Body.String())
	}

	var snapshot CapabilitySnapshot
	if err := json.Unmarshal(w.Body.Bytes(), &snapshot); err != nil {
		t.Fatalf("unmarshal snapshot failed: %v", err)
	}

	test := findSnapshotTest(&snapshot, "messages->messages")
	if test == nil {
		t.Fatalf("expected messages->messages in snapshot tests: %#v", snapshot.Tests)
	}
	if test.AttemptedModels == 0 || len(test.ModelResults) == 0 {
		t.Fatalf("expected model results for messages->messages, got attempted=%d len=%d", test.AttemptedModels, len(test.ModelResults))
	}
	mapped := false
	for _, result := range test.ModelResults {
		if result.Model == "claude-sonnet-4-6" && result.ActualModel == "claude-sonnet-4-6-upstream" {
			mapped = true
			break
		}
	}
	if !mapped {
		t.Fatalf("expected mapped claude-sonnet-4-6 model in messages->messages results: %#v", test.ModelResults)
	}
}

func TestFilterSameSourceVirtualProtocols(t *testing.T) {
	tests := []CapabilityProtocolJobResult{
		{Protocol: "responses->responses"},
		{Protocol: "responses->chat"},
		{Protocol: "responses"},
		{Protocol: "messages->messages"},
		{Protocol: "messages->chat"},
	}

	filtered := filterSameSourceVirtualProtocols(tests, "responses->chat")

	if findProtocolResult(filtered, "responses->responses") != nil {
		t.Fatal("expected responses->responses to be filtered")
	}
	if findProtocolResult(filtered, "messages->messages") != nil {
		t.Fatal("expected messages->messages to be filtered")
	}
	for _, protocol := range []string{"responses->chat", "responses", "messages->chat"} {
		if findProtocolResult(filtered, protocol) == nil {
			t.Fatalf("expected %s to be preserved", protocol)
		}
	}
}

func TestFilterSameSourceVirtualProtocols_PreservesCurrentProtocol(t *testing.T) {
	tests := []CapabilityProtocolJobResult{
		{Protocol: "messages->messages"},
		{Protocol: "chat->chat"},
		{Protocol: "responses->responses"},
		{Protocol: "gemini->gemini"},
		{Protocol: "messages->chat"},
	}

	for _, preserveProtocol := range []string{"messages->messages", "chat->chat", "responses->responses", "gemini->gemini"} {
		t.Run(preserveProtocol, func(t *testing.T) {
			filtered := filterSameSourceVirtualProtocols(tests, preserveProtocol)

			if findProtocolResult(filtered, preserveProtocol) == nil {
				t.Fatalf("expected %s to be preserved", preserveProtocol)
			}
			for _, protocol := range []string{"messages->messages", "chat->chat", "responses->responses", "gemini->gemini"} {
				if protocol != preserveProtocol && findProtocolResult(filtered, protocol) != nil {
					t.Fatalf("expected %s to be filtered", protocol)
				}
			}
			if findProtocolResult(filtered, "messages->chat") == nil {
				t.Fatal("expected messages->chat to be preserved")
			}
		})
	}
}

func findProtocolResult(tests []CapabilityProtocolJobResult, protocol string) *CapabilityProtocolJobResult {
	for i := range tests {
		if tests[i].Protocol == protocol {
			return &tests[i]
		}
	}
	return nil
}

func findSnapshotTest(snapshot *CapabilitySnapshot, protocol string) *CapabilityProtocolJobResult {
	for i := range snapshot.Tests {
		if snapshot.Tests[i].Protocol == protocol {
			return &snapshot.Tests[i]
		}
	}
	return nil
}

func TestCapabilitySnapshotStore_GCRemovesExpiredSnapshots(t *testing.T) {
	store := newCapabilitySnapshotStore()
	store.ttl = time.Hour
	store.snapshots["expired"] = &CapabilitySnapshot{
		IdentityKey: "expired",
		UpdatedAt:   time.Now().Add(-2 * time.Hour).Format(time.RFC3339Nano),
	}
	store.snapshots["fresh"] = &CapabilitySnapshot{
		IdentityKey: "fresh",
		UpdatedAt:   time.Now().Format(time.RFC3339Nano),
	}

	store.gc()

	if _, ok := store.get("expired"); ok {
		t.Fatal("expected expired snapshot to be removed")
	}
	if _, ok := store.get("fresh"); !ok {
		t.Fatal("expected fresh snapshot to remain")
	}
}

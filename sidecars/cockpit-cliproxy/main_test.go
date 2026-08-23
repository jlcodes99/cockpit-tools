package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/thinking"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

type responsesTerminalEventTestError struct {
	event []byte
}

func TestReadManifestCodexTokenAuthAcceptsAgentIdentityWithoutAccessToken(t *testing.T) {
	authDir := t.TempDir()
	path := filepath.Join(authDir, "agent.json")
	payload := map[string]any{
		"type":              "codex",
		"auth_mode":         "agentIdentity",
		"agent_runtime_id":  "runtime-test",
		"agent_private_key": "private-key-test",
		"account_id":        "team-test",
		"chatgpt_user_id":   "user-test",
		"email":             "agent@example.com",
	}
	data, err := json.Marshal(payload)
	if err != nil {
		t.Fatalf("marshal auth: %v", err)
	}
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatalf("write auth: %v", err)
	}
	auth, err := readManifestCodexTokenAuth(&accountSpec{
		ID:       "account-test",
		Email:    "agent@example.com",
		AuthID:   "agent.json",
		AuthKind: "agent_identity",
	}, authDir, path)
	if err != nil {
		t.Fatalf("read Agent Identity auth: %v", err)
	}
	if auth.Attributes[coreauth.AttributeAuthKind] != coreauth.AuthKindOAuth {
		t.Fatalf("auth kind = %q", auth.Attributes[coreauth.AttributeAuthKind])
	}
	if got, _ := auth.Metadata["auth_mode"].(string); got != "agentIdentity" {
		t.Fatalf("auth mode = %q", got)
	}
	if _, exists := auth.Metadata["access_token"]; exists {
		t.Fatal("Agent Identity must not fabricate access_token")
	}
}

func (e responsesTerminalEventTestError) Error() string { return "server overloaded" }
func (e responsesTerminalEventTestError) StatusCode() int {
	return http.StatusServiceUnavailable
}
func (e responsesTerminalEventTestError) ResponsesStreamEvent() []byte {
	return bytes.Clone(e.event)
}

func TestWriteStreamTerminalErrorForResponsesPreservesResponseFailed(t *testing.T) {
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	event := []byte(`{"type":"response.failed","response":{"status":"failed","error":{"type":"service_unavailable_error","code":"server_is_overloaded","message":"Our servers are currently overloaded"}}}`)

	writeStreamTerminalErrorForFormat(c, responsesTerminalEventTestError{event: event}, sdktranslator.FormatOpenAIResponse)

	body := recorder.Body.String()
	if !strings.Contains(body, "event: response.failed") {
		t.Fatalf("expected response.failed event name, got %q", body)
	}
	if !strings.Contains(body, `"type":"response.failed"`) {
		t.Fatalf("expected top-level response.failed type, got %q", body)
	}
}

func TestResponsesSSEFramerBuffersPartialJSONAcrossChunks(t *testing.T) {
	framer := newRelayStreamFramer(sdktranslator.FormatOpenAIResponse, "/v1/responses")
	var output strings.Builder

	first := []byte("event: response.completed\ndata: {\"type\":\"response.comp")
	if err := framer.Write(&output, first); err != nil {
		t.Fatalf("write first chunk: %v", err)
	}
	if output.Len() != 0 {
		t.Fatalf("partial JSON should remain buffered, got %q", output.String())
	}

	second := []byte("leted\",\"response\":{\"id\":\"resp_1\"}}")
	if err := framer.Write(&output, second); err != nil {
		t.Fatalf("write second chunk: %v", err)
	}
	if err := framer.Close(&output); err != nil {
		t.Fatalf("close framer: %v", err)
	}

	want := "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n"
	if got := output.String(); got != want {
		t.Fatalf("framed output = %q, want %q", got, want)
	}
}

func TestResponsesSSEFramerRepairsConcatenatedJSONDocuments(t *testing.T) {
	first := `{"type":"response.in_progress","response":{"id":"resp_1"}}`
	second := `{"type":"response.output_item.added","output_index":0,"item":{"type":"message"}}`

	tests := []struct {
		name  string
		chunk string
	}{
		{
			name:  "plain JSON chunk",
			chunk: first + second,
		},
		{
			name:  "SSE data line",
			chunk: "event: response.in_progress\ndata: " + first + second + "\n\n",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			framer := newRelayStreamFramer(sdktranslator.FormatOpenAIResponse, "/v1/responses")
			var output strings.Builder
			if err := framer.Write(&output, []byte(tc.chunk)); err != nil {
				t.Fatalf("write concatenated chunk: %v", err)
			}
			if err := framer.Close(&output); err != nil {
				t.Fatalf("close framer: %v", err)
			}

			frames := strings.Split(strings.TrimSpace(output.String()), "\n\n")
			wantTypes := []string{"response.in_progress", "response.output_item.added"}
			if len(frames) != len(wantTypes) {
				t.Fatalf("frame count = %d, want %d; output=%q", len(frames), len(wantTypes), output.String())
			}
			for i, frame := range frames {
				lines := strings.Split(frame, "\n")
				if len(lines) != 2 {
					t.Fatalf("frame %d lines = %d, want 2; frame=%q", i, len(lines), frame)
				}
				if got := strings.TrimSpace(strings.TrimPrefix(lines[0], "event:")); got != wantTypes[i] {
					t.Fatalf("frame %d event = %q, want %q", i, got, wantTypes[i])
				}
				data := strings.TrimSpace(strings.TrimPrefix(lines[1], "data:"))
				if !json.Valid([]byte(data)) {
					t.Fatalf("frame %d data is invalid JSON: %q", i, data)
				}
				var envelope struct {
					Type string `json:"type"`
				}
				if err := json.Unmarshal([]byte(data), &envelope); err != nil {
					t.Fatalf("decode frame %d: %v", i, err)
				}
				if envelope.Type != wantTypes[i] {
					t.Fatalf("frame %d payload type = %q, want %q", i, envelope.Type, wantTypes[i])
				}
			}
		})
	}
}

func TestSplitResponsesConcatenatedJSONDocumentsRejectsMalformedPayload(t *testing.T) {
	payload := []byte(`{"type":"response.in_progress"}{"missing_type":true}`)
	if documents, repaired := splitResponsesConcatenatedJSONDocuments(payload); repaired || documents != nil {
		t.Fatalf("malformed payload should not be repaired: %#v", documents)
	}
}

func TestCodexClientModelsResponseShape(t *testing.T) {
	response := buildCodexClientModelsResponse([]string{"gpt-5.4", "gpt-image-2", codexAutoReviewModel}, &apiKeySpec{}, nil)
	models, ok := response["models"].([]map[string]any)
	if !ok {
		t.Fatalf("models response should contain a models array: %#v", response["models"])
	}
	if len(models) != 3 {
		t.Fatalf("expected 3 models, got %d", len(models))
	}
	textModel := findCodexClientModelForTest(models, "gpt-5.4")
	imageModel := findCodexClientModelForTest(models, "gpt-image-2")
	reviewModel := findCodexClientModelForTest(models, codexAutoReviewModel)
	if textModel == nil || imageModel == nil || reviewModel == nil {
		t.Fatalf("expected all requested models, got %#v", models)
	}
	if got, ok := textModel["prefer_websockets"].(bool); !ok || got {
		t.Fatalf("text model prefer_websockets = %#v, want false by default", textModel["prefer_websockets"])
	}
	if textModel["visibility"] != "list" {
		t.Fatalf("text model should be listed in Codex client catalog: %#v", textModel)
	}
	if textModel["shell_type"] != "shell_command" || textModel["supported_in_api"] != true {
		t.Fatalf("text model should keep required Codex catalog fields: %#v", textModel)
	}
	if _, ok := textModel["input_modalities"].([]any); !ok {
		t.Fatalf("text model should keep input modalities: %#v", textModel)
	}
	// Official catalog service tiers / context must not be hard-cleared by main.go.
	if tiers, ok := textModel["service_tiers"].([]any); !ok || len(tiers) == 0 {
		t.Fatalf("text model should keep official service_tiers: %#v", textModel["service_tiers"])
	}
	if cw := intFromAny(textModel["max_context_window"]); cw != 1000000 {
		// gpt-5.4 template uses max_context_window=1000000; ensure we did not wipe it.
		t.Fatalf("text model max_context_window should keep template value 1000000, got %#v", textModel["max_context_window"])
	}
	if cw := intFromAny(textModel["context_window"]); cw != 272000 {
		t.Fatalf("text model context_window should keep template value 272000, got %#v", textModel["context_window"])
	}
	if imageModel["visibility"] != "hide" {
		t.Fatalf("image model should be hidden in Codex client catalog: %#v", imageModel)
	}
	if reviewModel["visibility"] != "hide" {
		t.Fatalf("auto review model should be hidden in Codex client catalog: %#v", reviewModel)
	}
}

func TestCodexClientModelsResponsePreserves56Template(t *testing.T) {
	response := buildCodexClientModelsResponse([]string{"gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "custom-compat-model"}, &apiKeySpec{}, nil)
	models, ok := response["models"].([]map[string]any)
	if !ok {
		t.Fatalf("models response should contain a models array: %#v", response["models"])
	}
	sol := findCodexClientModelForTest(models, "gpt-5.6-sol")
	if sol == nil {
		t.Fatal("expected gpt-5.6-sol")
	}
	if intFromAny(sol["context_window"]) != 372000 || intFromAny(sol["max_context_window"]) != 372000 {
		t.Fatalf("sol context windows = %#v / %#v", sol["context_window"], sol["max_context_window"])
	}
	if tiers, ok := sol["service_tiers"].([]any); !ok || len(tiers) != 1 {
		t.Fatalf("sol service_tiers = %#v", sol["service_tiers"])
	}
	if got, ok := sol["supports_search_tool"].(bool); !ok || !got {
		t.Fatalf("sol supports_search_tool = %#v, want true", sol["supports_search_tool"])
	}
	if got, ok := sol["prefer_websockets"].(bool); !ok || got {
		t.Fatalf("sol prefer_websockets = %#v, want false by default", sol["prefer_websockets"])
	}
	if got := stringFromAny(sol["minimal_client_version"]); got != "0.144.0" {
		t.Fatalf("sol minimal_client_version = %q, want 0.144.0", got)
	}
	levels, ok := sol["supported_reasoning_levels"].([]any)
	if !ok {
		t.Fatalf("sol reasoning levels = %#v", sol["supported_reasoning_levels"])
	}
	hasUltra := false
	for _, raw := range levels {
		level, _ := raw.(map[string]any)
		if stringFromAny(level["effort"]) == "ultra" {
			hasUltra = true
		}
	}
	if !hasUltra {
		t.Fatalf("sol should expose ultra reasoning: %#v", levels)
	}

	custom := findCodexClientModelForTest(models, "custom-compat-model")
	if custom == nil {
		t.Fatal("expected synthesized custom model")
	}
	if got, ok := custom["supports_search_tool"].(bool); !ok || got {
		t.Fatalf("custom supports_search_tool = %#v, want false", custom["supports_search_tool"])
	}
}

func TestCodexClientModelsResponseAppliesExplicitContextWindows(t *testing.T) {
	response := buildCodexClientModelsResponse(
		[]string{"gpt-5.4", "gpt-5.6-sol", "custom-flash"},
		&apiKeySpec{},
		map[string]int64{
			"gpt-5.6-sol":  900000,
			"custom-flash": 1048576,
		},
	)
	models, ok := response["models"].([]map[string]any)
	if !ok {
		t.Fatalf("models response should contain a models array: %#v", response["models"])
	}
	official := findCodexClientModelForTest(models, "gpt-5.4")
	sol := findCodexClientModelForTest(models, "gpt-5.6-sol")
	custom := findCodexClientModelForTest(models, "custom-flash")
	if official == nil || sol == nil || custom == nil {
		t.Fatalf("expected official, remapped, and custom models: %#v", models)
	}
	if intFromAny(official["context_window"]) != 272000 {
		t.Fatalf("official model should keep vendor context_window, got %#v", official["context_window"])
	}
	if intFromAny(sol["context_window"]) != 900000 || intFromAny(sol["max_context_window"]) != 900000 {
		t.Fatalf("explicit remap window = %#v / %#v", sol["context_window"], sol["max_context_window"])
	}
	if intFromAny(custom["context_window"]) != 1048576 || intFromAny(custom["max_context_window"]) != 1048576 {
		t.Fatalf("explicit custom window = %#v / %#v", custom["context_window"], custom["max_context_window"])
	}
}

func TestContextWindowsForAPIKeyMergesScopedAccounts(t *testing.T) {
	manifest := &manifest{
		Accounts: []accountSpec{
			{
				ID: "account-a",
				ModelContextWindows: map[string]int64{
					"gpt-5.6-sol": 900000,
				},
			},
			{
				ID: "account-b",
				ModelContextWindows: map[string]int64{
					"custom-flash": 1048576,
					"gpt-5.6-sol":  128000,
				},
			},
		},
	}
	manifest.accountByID = map[string]*accountSpec{
		"account-a": &manifest.Accounts[0],
		"account-b": &manifest.Accounts[1],
	}
	merged := contextWindowsForAPIKey(manifest, &apiKeySpec{AccountIDs: []string{"account-b"}})
	if merged["gpt-5.6-sol"] != 128000 || merged["custom-flash"] != 1048576 {
		t.Fatalf("scoped merge = %#v", merged)
	}
	if _, ok := contextWindowsForAPIKey(manifest, &apiKeySpec{AccountIDs: []string{"account-a"}})["custom-flash"]; ok {
		t.Fatal("account-a should not expose account-b windows")
	}
}

func TestCodexClientModelsResponseDoesNotInjectFastMode(t *testing.T) {
	for _, test := range []struct {
		name string
		spec *apiKeySpec
	}{
		{name: "plain API key", spec: &apiKeySpec{}},
		{name: "OAuth-bound API key", spec: &apiKeySpec{BoundOAuth: true}},
		{name: "provider gateway", spec: &apiKeySpec{ProviderGateway: &providerGatewaySpec{}}},
	} {
		t.Run(test.name, func(t *testing.T) {
			response := buildCodexClientModelsResponse([]string{"gpt-5.6-sol", "custom-compat-model"}, test.spec, nil)
			models, ok := response["models"].([]map[string]any)
			if !ok {
				t.Fatalf("models response should contain a models array: %#v", response["models"])
			}
			for _, slug := range []string{"gpt-5.6-sol", "custom-compat-model"} {
				model := findCodexClientModelForTest(models, slug)
				if model == nil {
					t.Fatalf("expected model %s", slug)
				}
				hasFast := false
				for _, raw := range model["service_tiers"].([]any) {
					tier, _ := raw.(map[string]any)
					id := strings.ToLower(strings.TrimSpace(stringFromAny(tier["id"])))
					name := strings.ToLower(strings.TrimSpace(stringFromAny(tier["name"])))
					if id == "priority" || id == "fast" || name == "fast" {
						hasFast = true
					}
				}
				wantFast := slug == "gpt-5.6-sol"
				if hasFast != wantFast {
					t.Fatalf("model %s Fast tier = %v, want %v: %#v", slug, hasFast, wantFast, model["service_tiers"])
				}
			}
		})
	}
}

func TestCodexClientModelsResponseEnablesWebsocketsWhenConfigured(t *testing.T) {
	response := buildCodexClientModelsResponse([]string{"gpt-5.6-sol"}, &apiKeySpec{
		ResponsesWebsockets: true,
	}, nil)
	models, ok := response["models"].([]map[string]any)
	if !ok {
		t.Fatalf("models response should contain a models array: %#v", response["models"])
	}
	sol := findCodexClientModelForTest(models, "gpt-5.6-sol")
	if sol == nil {
		t.Fatal("expected gpt-5.6-sol")
	}
	if got, ok := sol["prefer_websockets"].(bool); !ok || !got {
		t.Fatalf("sol prefer_websockets = %#v, want true", sol["prefer_websockets"])
	}
}

func TestBuildCockpitQuotaResponseAggregatesShortestWindowWithoutClamp(t *testing.T) {
	hourlyPresent := true
	weeklyPresent := true
	hourlyMinutes := int64(300)
	weeklyMinutes := int64(10080)
	accounts := map[string]quotaPoolAccountState{
		"team-1": {
			Primary: &quotaPoolWindowState{Present: &weeklyPresent, RemainingPercent: intPtrForTest(100), WindowMinutes: &weeklyMinutes},
		},
		"plus-with-hourly": {
			Primary:   &quotaPoolWindowState{Present: &hourlyPresent, RemainingPercent: intPtrForTest(80), WindowMinutes: &hourlyMinutes},
			Secondary: &quotaPoolWindowState{Present: &weeklyPresent, RemainingPercent: intPtrForTest(40), WindowMinutes: &weeklyMinutes},
		},
	}
	accountIDs := make([]string, 0, 16)
	for index := 0; index < 15; index++ {
		accountID := fmt.Sprintf("plus-%d", index+1)
		remaining := 86
		if index == 14 {
			remaining = 83
		}
		accounts[accountID] = quotaPoolAccountState{
			Primary: &quotaPoolWindowState{Present: &weeklyPresent, RemainingPercent: intPtrForTest(remaining), WindowMinutes: &weeklyMinutes},
		}
		accountIDs = append(accountIDs, accountID)
	}
	accountIDs = append(accountIDs, "team-1")
	state := quotaPoolStateFile{Accounts: accounts}
	response := buildCockpitQuotaResponse(&apiKeySpec{AccountIDs: accountIDs}, state, time.Now())
	if response.RemainingPercent == nil || *response.RemainingPercent != 1387 {
		t.Fatalf("remaining percent = %#v, want 1387", response.RemainingPercent)
	}
	if response.WeeklyRemainingPercent == nil || *response.WeeklyRemainingPercent != 1387 {
		t.Fatalf("weekly percent = %#v, want 1387", response.WeeklyRemainingPercent)
	}
	if response.FiveHourRemainingPercent != nil {
		t.Fatalf("five-hour percent should be absent: %#v", response.FiveHourRemainingPercent)
	}
	if response.IncludedAccountCount != 16 || response.MissingAccountCount != 0 {
		t.Fatalf("account counts = %d/%d, want 16/0", response.IncludedAccountCount, response.MissingAccountCount)
	}
	shortest := buildCockpitQuotaResponse(&apiKeySpec{AccountIDs: []string{"plus-with-hourly"}}, state, time.Now())
	if shortest.RemainingPercent == nil || *shortest.RemainingPercent != 80 {
		t.Fatalf("shortest window percent = %#v, want 80", shortest.RemainingPercent)
	}
	if shortest.WeeklyRemainingPercent == nil || *shortest.WeeklyRemainingPercent != 40 {
		t.Fatalf("weekly percent = %#v, want 40", shortest.WeeklyRemainingPercent)
	}
	if shortest.FiveHourRemainingPercent == nil || *shortest.FiveHourRemainingPercent != 80 {
		t.Fatalf("five-hour percent = %#v, want 80", shortest.FiveHourRemainingPercent)
	}
	emptyScope := buildCockpitQuotaResponse(&apiKeySpec{}, state, time.Now())
	if emptyScope.RemainingPercent != nil || emptyScope.AccountCount != 0 {
		t.Fatalf("empty API key scope must not expose the full quota pool: %#v", emptyScope)
	}
}

func TestBuildCockpitQuotaResponseGroupsPlansAndPoolHealth(t *testing.T) {
	present := true
	fiveHourMinutes := int64(300)
	weeklyMinutes := int64(10080)
	state := quotaPoolStateFile{Accounts: map[string]quotaPoolAccountState{
		"plus-1": {
			Primary:   &quotaPoolWindowState{Present: &present, RemainingPercent: intPtrForTest(80), WindowMinutes: &fiveHourMinutes},
			Secondary: &quotaPoolWindowState{Present: &present, RemainingPercent: intPtrForTest(40), WindowMinutes: &weeklyMinutes},
		},
		"team-1": {
			Primary: &quotaPoolWindowState{Present: &present, RemainingPercent: intPtrForTest(75), WindowMinutes: &weeklyMinutes},
		},
	}}
	accounts := map[string]*accountSpec{
		"plus-1":    {ID: "plus-1", PlanType: "plus"},
		"plus-2":    {ID: "plus-2", PlanType: "plus"},
		"team-1":    {ID: "team-1", PlanType: "team"},
		"api-key-1": {ID: "api-key-1", AuthKind: "api_key", PlanType: "custom"},
	}
	response := buildCockpitQuotaResponseWithAccounts(
		&apiKeySpec{AccountIDs: []string{"plus-1", "plus-2", "team-1", "api-key-1"}},
		state,
		time.Now(),
		accounts,
	)
	if response.AccountCount != 3 || response.AvailableAccountCount != 2 || response.AbnormalAccountCount != 1 || response.CooldownAccountCount != 0 {
		t.Fatalf("pool health = available %d, abnormal %d, cooldown %d", response.AvailableAccountCount, response.AbnormalAccountCount, response.CooldownAccountCount)
	}
	if len(response.Plans) != 3 {
		t.Fatalf("plan summaries = %#v, want 3 groups", response.Plans)
	}
	if response.Plans[0].Plan != "PLUS" || response.Plans[0].Count != 2 || response.Plans[0].WeeklyRemainingPercent == nil || *response.Plans[0].WeeklyRemainingPercent != 40 || response.Plans[0].FiveHourRemainingPercent == nil || *response.Plans[0].FiveHourRemainingPercent != 80 {
		t.Fatalf("PLUS summary = %#v", response.Plans[0])
	}
	if response.Plans[1].Plan != "TEAM" || response.Plans[1].Count != 1 || response.Plans[1].WeeklyRemainingPercent == nil || *response.Plans[1].WeeklyRemainingPercent != 75 {
		t.Fatalf("TEAM summary = %#v", response.Plans[1])
	}
	if response.Plans[2].Plan != "API_KEY" || response.Plans[2].Count != 1 || response.Plans[2].WeeklyRemainingPercent != nil || response.Plans[2].FiveHourRemainingPercent != nil {
		t.Fatalf("API_KEY summary = %#v", response.Plans[2])
	}
	applyCockpitQuotaAuthHealth(&response, &apiKeySpec{AccountIDs: []string{"plus-1", "plus-2", "team-1", "api-key-1"}}, state, []*coreauth.Auth{{
		ID:             "team-auth",
		Unavailable:    true,
		NextRetryAfter: time.Now().Add(time.Minute),
		Attributes:     map[string]string{"account_id": "team-1"},
	}}, time.Now())
	if response.AvailableAccountCount != 1 || response.AbnormalAccountCount != 1 || response.CooldownAccountCount != 1 {
		t.Fatalf("runtime pool health = available %d, abnormal %d, cooldown %d", response.AvailableAccountCount, response.AbnormalAccountCount, response.CooldownAccountCount)
	}
}

func TestRelayServerCockpitQuotaRequiresKeyAndIsolatesAccountScopes(t *testing.T) {
	gin.SetMode(gin.TestMode)
	statePath := filepath.Join(t.TempDir(), "quota-pool-state.json")
	present := true
	minutes := int64(300)
	state := quotaPoolStateFile{Accounts: map[string]quotaPoolAccountState{
		"account-a": {
			Primary: &quotaPoolWindowState{Present: &present, RemainingPercent: intPtrForTest(80), WindowMinutes: &minutes},
		},
		"account-b": {
			Primary: &quotaPoolWindowState{Present: &present, RemainingPercent: intPtrForTest(55), WindowMinutes: &minutes},
		},
	}}
	content, err := json.Marshal(state)
	if err != nil {
		t.Fatalf("marshal quota state: %v", err)
	}
	if err := os.WriteFile(statePath, content, 0o600); err != nil {
		t.Fatalf("write quota state: %v", err)
	}

	keyA := &apiKeySpec{ID: "key-a", Key: "client-a", Enabled: true, AccountIDs: []string{"account-a"}}
	keyB := &apiKeySpec{ID: "key-b", Key: "client-b", Enabled: true, AccountIDs: []string{"account-b"}}
	manifest := &manifest{apiKeyByValue: map[string]*apiKeySpec{
		keyA.Key: keyA,
		keyB.Key: keyB,
	}}
	router := (&relayServer{
		runtime:            &fakeRuntime{},
		cfg:                &config.Config{},
		manifest:           manifest,
		policy:             &requestPolicy{manifest: manifest},
		quotaPoolStatePath: statePath,
	}).router()

	for _, key := range []string{"", "wrong-key"} {
		req := httptest.NewRequest(http.MethodGet, cockpitQuotaPath, nil)
		if key != "" {
			req.Header.Set("Authorization", "Bearer "+key)
		}
		w := httptest.NewRecorder()
		router.ServeHTTP(w, req)
		if w.Code != http.StatusUnauthorized {
			t.Fatalf("key %q status = %d, want 401; body=%s", key, w.Code, w.Body.String())
		}
	}

	for key, want := range map[string]int{"client-a": 80, "client-b": 55} {
		req := httptest.NewRequest(http.MethodGet, cockpitQuotaPath, nil)
		req.Header.Set("Authorization", "Bearer "+key)
		w := httptest.NewRecorder()
		router.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("key %q status = %d, want 200; body=%s", key, w.Code, w.Body.String())
		}
		var response cockpitQuotaResponse
		if err := json.Unmarshal(w.Body.Bytes(), &response); err != nil {
			t.Fatalf("decode key %q response: %v", key, err)
		}
		if response.RemainingPercent == nil || *response.RemainingPercent != want || response.AccountCount != 1 {
			t.Fatalf("key %q received another scope: %#v", key, response)
		}
	}
}

func TestRelayServerCockpitQuotaUpstreamFailureReturnsScopedEmptyState(t *testing.T) {
	gin.SetMode(gin.TestMode)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		http.Error(w, "unavailable", http.StatusBadGateway)
	}))
	defer upstream.Close()

	statePath := filepath.Join(t.TempDir(), "quota-pool-state.json")
	if err := os.WriteFile(statePath, []byte(`{"accounts":{}}`), 0o600); err != nil {
		t.Fatalf("write quota state: %v", err)
	}
	spec := &apiKeySpec{
		ID:         "provider-key",
		Key:        "client-key",
		Enabled:    true,
		AccountIDs: []string{"provider-account"},
		ProviderGateway: &providerGatewaySpec{
			BaseURL: upstream.URL,
			APIKey:  "upstream-key",
		},
	}
	manifest := &manifest{apiKeyByValue: map[string]*apiKeySpec{spec.Key: spec}}
	router := (&relayServer{
		runtime:            &fakeRuntime{},
		cfg:                &config.Config{},
		manifest:           manifest,
		policy:             &requestPolicy{manifest: manifest},
		quotaPoolStatePath: statePath,
	}).router()

	req := httptest.NewRequest(http.MethodGet, cockpitQuotaPath, nil)
	req.Header.Set("Authorization", "Bearer client-key")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", w.Code, w.Body.String())
	}
	var response cockpitQuotaResponse
	if err := json.Unmarshal(w.Body.Bytes(), &response); err != nil {
		t.Fatalf("decode response: %v", err)
	}
	if response.RemainingPercent != nil || response.AccountCount != 1 || response.MissingAccountCount != 1 {
		t.Fatalf("upstream failure should keep local scoped empty state: %#v", response)
	}
}

func TestCodexClientModelsResponseDisablesSearchForProviderGateway(t *testing.T) {
	response := buildCodexClientModelsResponse([]string{"gpt-5.6-sol"}, &apiKeySpec{
		ProviderGateway: &providerGatewaySpec{},
	}, nil)
	models, ok := response["models"].([]map[string]any)
	if !ok {
		t.Fatalf("models response should contain a models array: %#v", response["models"])
	}
	sol := findCodexClientModelForTest(models, "gpt-5.6-sol")
	if sol == nil {
		t.Fatal("expected gpt-5.6-sol")
	}
	if got, ok := sol["supports_search_tool"].(bool); !ok || got {
		t.Fatalf("provider gateway supports_search_tool = %#v, want false", sol["supports_search_tool"])
	}
}

func TestCodexClientModelsResponseGatesProviderGatewayImageInput(t *testing.T) {
	tests := []struct {
		name          string
		gateway       *providerGatewaySpec
		model         string
		supportsImage bool
	}{
		{
			name: "text only",
			gateway: &providerGatewaySpec{
				UpstreamModels: []string{"deepseek-v4-pro"},
			},
			model: "deepseek-v4-pro",
		},
		{
			name: "model supports vision",
			gateway: &providerGatewaySpec{
				UpstreamModels: []string{"qwen-vl-plus"},
				ModelCapabilities: map[string]providerGatewayModelCapability{
					"qwen-vl-plus": {SupportsVision: true},
				},
			},
			model:         "qwen-vl-plus",
			supportsImage: true,
		},
		{
			name: "routes images to vision model",
			gateway: &providerGatewaySpec{
				UpstreamModels:     []string{"deepseek-v4-pro", "qwen-vl-plus"},
				VisionRoutingModel: "qwen-vl-plus",
				ModelCapabilities: map[string]providerGatewayModelCapability{
					"qwen-vl-plus": {SupportsVision: true},
				},
			},
			model:         "deepseek-v4-pro",
			supportsImage: true,
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			response := buildCodexClientModelsResponse([]string{test.model}, &apiKeySpec{
				ProviderGateway: test.gateway,
			}, nil)
			models, ok := response["models"].([]map[string]any)
			if !ok {
				t.Fatalf("models response should contain a models array: %#v", response["models"])
			}
			entry := findCodexClientModelForTest(models, test.model)
			if entry == nil {
				t.Fatalf("expected model %s", test.model)
			}
			modalities, ok := entry["input_modalities"].([]any)
			if !ok {
				t.Fatalf("input_modalities = %#v", entry["input_modalities"])
			}
			want := []any{"text"}
			if test.supportsImage {
				want = []any{"text", "image"}
			}
			if !reflect.DeepEqual(modalities, want) {
				t.Fatalf("input_modalities = %#v, want %#v", modalities, want)
			}
			_, hasImageDetail := entry["supports_image_detail_original"]
			if hasImageDetail != test.supportsImage {
				t.Fatalf("supports_image_detail_original present = %v, want %v", hasImageDetail, test.supportsImage)
			}
		})
	}
}

func TestProviderGatewayVisionDetectionIgnoresToolSchemaFieldNames(t *testing.T) {
	body := []byte(`{
		"model":"deepseek-v4-pro",
		"tools":[{
			"type":"function",
			"name":"inspect_url",
			"parameters":{
				"type":"object",
				"properties":{"image_url":{"type":"string"}}
			}
		}]
	}`)
	if providerGatewayRequestHasVisionInput(body) {
		t.Fatal("tool schema field names must not be treated as image input")
	}
}

func intFromAny(value any) int {
	switch v := value.(type) {
	case int:
		return v
	case int64:
		return int(v)
	case float64:
		return int(v)
	default:
		return 0
	}
}

func stringFromAny(value any) string {
	if s, ok := value.(string); ok {
		return s
	}
	return ""
}

func TestCodexSparkUsesCompleteCodexClientCatalogTemplate(t *testing.T) {
	response := buildCodexClientModelsResponse([]string{codexSparkCatalogTemplateModel, codexSparkModel}, &apiKeySpec{}, nil)
	models, ok := response["models"].([]map[string]any)
	if !ok {
		t.Fatalf("models response should contain a models array: %#v", response["models"])
	}
	template := findCodexClientModelForTest(models, codexSparkCatalogTemplateModel)
	spark := findCodexClientModelForTest(models, codexSparkModel)
	if template == nil || spark == nil {
		t.Fatalf("expected template and Spark models, got %#v", models)
	}
	if spark["display_name"] != "GPT-5.3 Codex Spark" || spark["visibility"] != "list" || spark["supported_in_api"] != true {
		t.Fatalf("Spark should be listed as an API model: %#v", spark)
	}
	for _, field := range []string{"available_in_plans", "base_instructions", "minimal_client_version", "model_messages", "prefer_websockets"} {
		if spark[field] == nil || !reflect.DeepEqual(spark[field], template[field]) {
			t.Fatalf("Spark should inherit %s from the Codex client template: %#v", field, spark[field])
		}
	}
}

func findCodexClientModelForTest(models []map[string]any, slug string) map[string]any {
	for _, model := range models {
		if model["slug"] == slug {
			return model
		}
	}
	return nil
}

func TestVisibleModelsForAPIKeyUsesPrefixAndFilters(t *testing.T) {
	spec := &apiKeySpec{
		ModelPrefix:    "team",
		AllowedModels:  []string{"gpt-*"},
		ExcludedModels: []string{"gpt-image-*"},
	}
	m := &manifest{
		ModelIDs: []string{"gpt-5.4", "gpt-image-2", "custom-model"},
	}

	models := visibleModelsForAPIKey(m, spec)

	if len(models) != 1 || models[0] != "team/gpt-5.4" {
		t.Fatalf("unexpected visible models: %#v", models)
	}
}

func TestClientCatalogModelsIncludesAutoReviewWithoutPrefix(t *testing.T) {
	spec := &apiKeySpec{
		ModelPrefix:    "team",
		AllowedModels:  []string{"gpt-*"},
		ExcludedModels: []string{"gpt-image-*"},
	}
	m := &manifest{
		ModelIDs: []string{"gpt-5.4", "gpt-image-2", "custom-model"},
	}

	models := clientCatalogModelsForAPIKey(m, spec)

	if len(models) != 2 || models[0] != "team/gpt-5.4" || models[1] != codexAutoReviewModel {
		t.Fatalf("unexpected client catalog models: %#v", models)
	}
}

func TestCockpitSelectorRestrictsAuthsToClientAPIKeyAccountScope(t *testing.T) {
	highQuotaAccount := &accountSpec{
		ID:       "account-high",
		AuthID:   "account-high.json",
		PlanRank: intPtrForTest(500),
	}
	scopedAccount := &accountSpec{
		ID:       "account-scoped",
		AuthID:   "account-scoped.json",
		PlanRank: intPtrForTest(300),
	}
	selector := &cockpitSelector{
		manifest: &manifest{
			RoutingStrategy: "auto",
			accountByAuthID: map[string]*accountSpec{
				"account-high.json":   highQuotaAccount,
				"account-scoped.json": scopedAccount,
			},
			accountByID: map[string]*accountSpec{
				"account-high":   highQuotaAccount,
				"account-scoped": scopedAccount,
			},
		},
	}
	apiKey := &apiKeySpec{
		ID:         "key-scoped",
		Label:      "Scoped client",
		AccountIDs: []string{"account-scoped"},
	}
	ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, apiKey)
	auths := []*coreauth.Auth{
		{ID: "account-high.json", Provider: "codex", Status: coreauth.StatusActive},
		{ID: "account-scoped.json", Provider: "codex", Status: coreauth.StatusActive},
	}

	selected, err := selector.Pick(ctx, "codex", "gpt-5.6-sol", cliproxyexecutor.Options{}, auths)

	if err != nil {
		t.Fatalf("pick scoped auth: %v", err)
	}
	if selected.ID != "account-scoped.json" {
		t.Fatalf("expected only scoped account to be selected, got %q", selected.ID)
	}
}

func TestAPIKeyPriorityStateOrdersFallbackAccountsWithoutRestart(t *testing.T) {
	tempDir := t.TempDir()
	priorityPath := filepath.Join(tempDir, "api-key-priorities.json")
	if err := os.WriteFile(priorityPath, []byte(`{"priorityAccountIds":{"key-team":["account-a","account-b"]}}`), 0o600); err != nil {
		t.Fatalf("write priority state: %v", err)
	}
	store := newAPIKeyPriorityStateStore(filepath.Join(tempDir, "manifest.json"))

	accountA := &accountSpec{ID: "account-a"}
	accountB := &accountSpec{ID: "account-b"}
	accountC := &accountSpec{ID: "account-c"}
	selector := &cockpitSelector{
		manifest: &manifest{
			accountByAuthID: map[string]*accountSpec{
				"auth-a": accountA,
				"auth-b": accountB,
				"auth-c": accountC,
			},
		},
		priorities: store,
	}
	ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, &apiKeySpec{ID: "key-team"})
	auths := []*coreauth.Auth{{ID: "auth-c"}, {ID: "auth-b"}, {ID: "auth-a"}}
	ordered := selector.prioritizeAuthsForAPIKey(ctx, auths)
	if ordered[0].ID != "auth-a" || ordered[1].ID != "auth-b" || ordered[2].ID != "auth-c" {
		t.Fatalf("priority accounts should lead in order, got %#v", ordered)
	}
	fallbackAuths := []*coreauth.Auth{{ID: "auth-c"}, {ID: "auth-b"}}
	ordered = selector.prioritizeAuthsForAPIKey(ctx, fallbackAuths)
	if ordered[0].ID != "auth-b" {
		t.Fatalf("next priority account should lead when the first is unavailable, got %#v", ordered)
	}

	if err := os.WriteFile(priorityPath, []byte(`{"priorityAccountIds":{"key-team":["account-b","account-a"]}}`), 0o600); err != nil {
		t.Fatalf("update priority state: %v", err)
	}
	updatedAt := time.Now().Add(time.Second)
	if err := os.Chtimes(priorityPath, updatedAt, updatedAt); err != nil {
		t.Fatalf("advance priority state timestamp: %v", err)
	}
	ordered = selector.prioritizeAuthsForAPIKey(ctx, auths)
	if ordered[0].ID != "auth-b" || ordered[1].ID != "auth-a" {
		t.Fatalf("updated priority should apply without a sidecar restart, got %#v", ordered)
	}
}

func TestCockpitSessionAffinitySeparatesClientAPIKeyScopes(t *testing.T) {
	highQuotaAccount := &accountSpec{
		ID:       "account-high",
		AuthID:   "account-high.json",
		PlanRank: intPtrForTest(500),
	}
	scopedAccount := &accountSpec{
		ID:       "account-scoped",
		AuthID:   "account-scoped.json",
		PlanRank: intPtrForTest(300),
	}
	fallback := &cockpitSelector{
		manifest: &manifest{
			RoutingStrategy: "auto",
			accountByAuthID: map[string]*accountSpec{
				"account-high.json":   highQuotaAccount,
				"account-scoped.json": scopedAccount,
			},
			accountByID: map[string]*accountSpec{
				"account-high":   highQuotaAccount,
				"account-scoped": scopedAccount,
			},
		},
	}
	selector := &cockpitSessionAffinitySelector{
		inner: coreauth.NewSessionAffinitySelectorWithConfig(coreauth.SessionAffinityConfig{
			Fallback: fallback,
			TTL:      time.Hour,
		}),
	}
	auths := []*coreauth.Auth{
		{ID: "account-high.json", Provider: "codex", Status: coreauth.StatusActive},
		{ID: "account-scoped.json", Provider: "codex", Status: coreauth.StatusActive},
	}
	opts := cliproxyexecutor.Options{
		Headers: http.Header{"X-Session-ID": []string{"shared-session"}},
	}
	defaultKey := &apiKeySpec{
		ID:         "default-key",
		AccountIDs: []string{"account-high", "account-scoped"},
	}
	scopedKey := &apiKeySpec{
		ID:         "scoped-key",
		AccountIDs: []string{"account-scoped"},
	}

	first, err := selector.Pick(
		context.WithValue(context.Background(), clientAPIKeyContextKey, defaultKey),
		"codex",
		"gpt-5.4",
		opts,
		auths,
	)
	if err != nil {
		t.Fatalf("pick default key auth: %v", err)
	}
	if first.ID != "account-high.json" {
		t.Fatalf("expected default key to select high quota auth, got %q", first.ID)
	}

	second, err := selector.Pick(
		context.WithValue(context.Background(), clientAPIKeyContextKey, scopedKey),
		"codex",
		"gpt-5.4",
		opts,
		auths,
	)
	if err != nil {
		t.Fatalf("pick scoped key auth: %v", err)
	}
	if second.ID != "account-scoped.json" {
		t.Fatalf("expected scoped key not to reuse default key affinity auth, got %q", second.ID)
	}
}

func intPtrForTest(value int) *int {
	return &value
}

func TestCanonicalModelForClientModelHandlesPrefixAliasAndSnapshot(t *testing.T) {
	spec := &apiKeySpec{ModelPrefix: "team"}
	m := &manifest{
		ModelIDs:      []string{"gpt-5.4", "gpt-5.4-mini"},
		aliasToSource: map[string]string{"fast": "gpt-5.4-mini"},
	}

	if got := canonicalModelForClientModel(m, spec, "team/fast"); got != "gpt-5.4-mini" {
		t.Fatalf("alias should resolve to source model, got %q", got)
	}
	if got := canonicalModelForClientModel(m, spec, "team/gpt-5.4-2026-03-05"); got != "gpt-5.4" {
		t.Fatalf("snapshot should resolve to supported model, got %q", got)
	}
	if got := canonicalModelForClientModel(m, spec, codexAutoReviewModel); got != codexAutoReviewModel {
		t.Fatalf("auto review model should stay canonical, got %q", got)
	}
}

func TestLoadManifestIndexesAPIKeyAccounts(t *testing.T) {
	path := filepath.Join(t.TempDir(), "manifest.json")
	if err := os.WriteFile(path, []byte(`{
		"apiKeys": [{"id":"client","label":"Client","key":"client-key","enabled":true}],
		"accounts": [{"id":"api-account","email":"api@example.com","upstreamApiKey":"  sk-upstream  "}]
	}`), 0o644); err != nil {
		t.Fatalf("write manifest: %v", err)
	}

	m, err := loadManifest(path)
	if err != nil {
		t.Fatalf("load manifest: %v", err)
	}

	account := m.accountByAPIKey["sk-upstream"]
	if account == nil {
		t.Fatalf("API Key account should be indexed by upstream key: %#v", m.accountByAPIKey)
	}
	if account.ID != "api-account" || account.UpstreamAPIKey != "sk-upstream" {
		t.Fatalf("unexpected indexed account: %#v", account)
	}
}

func TestLoadManifestIndexesTokenAccounts(t *testing.T) {
	path := filepath.Join(t.TempDir(), "manifest.json")
	if err := os.WriteFile(path, []byte(`{
		"accounts": [{
			"id":"token-account",
			"email":" token@example.com ",
			"authId":"nested/token-account.json",
			"authKind":"access_token",
			"accessTokenOnly":true,
			"chatgptAccountId":" acct-token "
		}]
	}`), 0o644); err != nil {
		t.Fatalf("write manifest: %v", err)
	}

	m, err := loadManifest(path)
	if err != nil {
		t.Fatalf("load manifest: %v", err)
	}

	if got := m.accountByAuthID["nested/token-account.json"]; got == nil || got.ID != "token-account" {
		t.Fatalf("auth id should index token account, got %#v", got)
	}
	if got := m.accountByAuthID["token-account.json"]; got == nil || got.ID != "token-account" {
		t.Fatalf("auth file basename should index token account, got %#v", got)
	}
	if got := m.accountByChatGPT["acct-token"]; got == nil || got.ID != "token-account" {
		t.Fatalf("chatgpt account id should index token account, got %#v", got)
	}
	if got := m.accountByEmail["token@example.com"]; got == nil || got.ID != "token-account" {
		t.Fatalf("email should index token account, got %#v", got)
	}
}

func TestLoadManifestParsesBoundOAuthQuotaReserve(t *testing.T) {
	path := filepath.Join(t.TempDir(), "manifest.json")
	if err := os.WriteFile(path, []byte(`{
		"accounts": [{
			"id": "oauth-account",
			"email": "oauth@example.com",
			"authId": "oauth-account.json",
			"quotaReserve": {
				"hourlyThresholdPercent": 10,
				"weeklyThresholdPercent": 20,
				"snapshotUpdatedAtUnixSeconds": 1234567890,
				"hourlyRemainingPercent": 55,
				"weeklyRemainingPercent": 66,
				"hourlyWindowPresent": true,
				"weeklyWindowPresent": false
			}
		}]
	}`), 0o644); err != nil {
		t.Fatalf("write manifest: %v", err)
	}

	m, err := loadManifest(path)
	if err != nil {
		t.Fatalf("load manifest: %v", err)
	}
	account := m.accountByID["oauth-account"]
	if account == nil || account.QuotaReserve == nil {
		t.Fatalf("quota reserve should be parsed: %#v", account)
	}
	reserve := account.QuotaReserve
	if reserve.HourlyThresholdPercent == nil || *reserve.HourlyThresholdPercent != 10 ||
		reserve.WeeklyThresholdPercent == nil || *reserve.WeeklyThresholdPercent != 20 ||
		reserve.SnapshotUpdatedAtUnixSeconds == nil || *reserve.SnapshotUpdatedAtUnixSeconds != 1234567890 ||
		reserve.HourlyRemainingPercent == nil || *reserve.HourlyRemainingPercent != 55 ||
		reserve.WeeklyRemainingPercent == nil || *reserve.WeeklyRemainingPercent != 66 ||
		reserve.HourlyWindowPresent == nil || !*reserve.HourlyWindowPresent ||
		reserve.WeeklyWindowPresent == nil || *reserve.WeeklyWindowPresent {
		t.Fatalf("unexpected parsed quota reserve: %#v", reserve)
	}
}

func TestCockpitSelectorPickSkipsBoundOAuthAtEitherQuotaReserve(t *testing.T) {
	tests := []struct {
		name            string
		hourlyRemaining int
		weeklyRemaining int
	}{
		{name: "hourly", hourlyRemaining: 10, weeklyRemaining: 90},
		{name: "weekly", hourlyRemaining: 90, weeklyRemaining: 20},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			hourlyThreshold := 10
			weeklyThreshold := 20
			snapshotUpdatedAt := time.Now().Unix()
			windowPresent := true
			protectedAccount := &accountSpec{
				ID:     "protected",
				Email:  "protected@example.com",
				AuthID: "protected.json",
				QuotaReserve: &quotaReserveSpec{
					HourlyThresholdPercent:       &hourlyThreshold,
					WeeklyThresholdPercent:       &weeklyThreshold,
					SnapshotUpdatedAtUnixSeconds: &snapshotUpdatedAt,
					HourlyRemainingPercent:       &tt.hourlyRemaining,
					WeeklyRemainingPercent:       &tt.weeklyRemaining,
					HourlyWindowPresent:          &windowPresent,
					WeeklyWindowPresent:          &windowPresent,
				},
			}
			normalAccount := &accountSpec{ID: "normal", AuthID: "normal.json"}
			selector := &cockpitSelector{manifest: &manifest{
				accountByAuthID: map[string]*accountSpec{
					"protected.json": protectedAccount,
					"normal.json":    normalAccount,
				},
			}}

			selected, err := selector.Pick(
				context.Background(),
				"codex",
				"gpt-5.4",
				cliproxyexecutor.Options{},
				[]*coreauth.Auth{{ID: "protected.json"}, {ID: "normal.json"}},
			)
			if err != nil {
				t.Fatalf("Pick: %v", err)
			}
			if selected == nil || selected.ID != "normal.json" {
				t.Fatalf("expected normal auth after reserve filtering, got %#v", selected)
			}
		})
	}
}

func TestCockpitSelectorPrefersAccountWithFewerImageJobs(t *testing.T) {
	busyAccount := &accountSpec{ID: "busy", AuthID: "busy.json"}
	idleAccount := &accountSpec{ID: "idle", AuthID: "idle.json"}
	tracker := newRequestUsageTracker()
	if !tracker.tryReserveImageJob("existing-image", "busy.json", 1) {
		t.Fatal("expected initial busy image reservation")
	}
	if tracker.tryReserveImageJob("competing-image", "busy.json", 1) {
		t.Fatal("expected busy image auth to reject a second concurrent reservation")
	}
	selector := &cockpitSelector{
		manifest: &manifest{accountByAuthID: map[string]*accountSpec{
			"busy.json": busyAccount,
			"idle.json": idleAccount,
		}},
		tracker: tracker,
	}
	ctx := internallogging.WithRequestID(context.Background(), "new-image")
	ctx = context.WithValue(ctx, requestKindContextKey, "image_generation")

	selected, err := selector.Pick(
		ctx,
		"codex",
		"gpt-5.4-mini",
		cliproxyexecutor.Options{},
		[]*coreauth.Auth{{ID: "busy.json"}, {ID: "idle.json"}},
	)
	if err != nil {
		t.Fatalf("Pick: %v", err)
	}
	if selected == nil || selected.ID != "idle.json" {
		t.Fatalf("expected idle auth, got %#v", selected)
	}
	if got := tracker.imageInFlightCount("idle.json"); got != 1 {
		t.Fatalf("idle auth in-flight count = %d, want 1", got)
	}

	changed := tracker.imageJobChangeSignal()
	tracker.releaseImageJobs("new-image")
	select {
	case <-changed:
	default:
		t.Fatal("expected image slot release notification")
	}
	if got := tracker.imageInFlightCount("idle.json"); got != 0 {
		t.Fatalf("idle auth in-flight count after release = %d, want 0", got)
	}
}

func TestRequestUsageTrackerHonorsConfiguredImageJobLimit(t *testing.T) {
	tracker := newRequestUsageTracker()
	if !tracker.tryReserveImageJob("first-image", "shared.json", 2) {
		t.Fatal("expected first image reservation")
	}
	if !tracker.tryReserveImageJob("second-image", "shared.json", 2) {
		t.Fatal("expected second image reservation within configured limit")
	}
	if tracker.tryReserveImageJob("third-image", "shared.json", 2) {
		t.Fatal("expected image reservation above configured limit to be rejected")
	}
	if got := tracker.imageInFlightCount("shared.json"); got != 2 {
		t.Fatalf("shared auth in-flight count = %d, want 2", got)
	}
}

func TestImageRequestSelectorBypassesSessionAffinityFallback(t *testing.T) {
	imageAuth := &coreauth.Auth{ID: "image.json"}
	affinityAuth := &coreauth.Auth{ID: "affinity.json"}
	imageFallback := &countingSelector{auth: imageAuth}
	affinityFallback := &countingSelector{auth: affinityAuth}
	selector := &imageRequestSelector{
		imageFallback: imageFallback,
		fallback:      affinityFallback,
	}
	ctx := context.WithValue(context.Background(), requestKindContextKey, "image_generation")

	selected, err := selector.Pick(ctx, "codex", "gpt-5.4-mini", cliproxyexecutor.Options{}, []*coreauth.Auth{imageAuth, affinityAuth})
	if err != nil {
		t.Fatalf("Pick: %v", err)
	}
	if selected != imageAuth || imageFallback.count != 1 || affinityFallback.count != 0 {
		t.Fatalf("image request should use image fallback, selected=%#v image=%d affinity=%d", selected, imageFallback.count, affinityFallback.count)
	}
}

func TestCockpitSelectorPickIgnoresExplicitlyMissingQuotaWindow(t *testing.T) {
	hourlyThreshold := 10
	weeklyThreshold := 20
	snapshotUpdatedAt := time.Now().Unix()
	weeklyRemaining := 80
	hourlyWindowPresent := false
	weeklyWindowPresent := true
	account := &accountSpec{
		ID:     "protected",
		AuthID: "protected.json",
		QuotaReserve: &quotaReserveSpec{
			HourlyThresholdPercent:       &hourlyThreshold,
			WeeklyThresholdPercent:       &weeklyThreshold,
			SnapshotUpdatedAtUnixSeconds: &snapshotUpdatedAt,
			HourlyRemainingPercent:       nil,
			WeeklyRemainingPercent:       &weeklyRemaining,
			HourlyWindowPresent:          &hourlyWindowPresent,
			WeeklyWindowPresent:          &weeklyWindowPresent,
		},
	}
	selector := &cockpitSelector{manifest: &manifest{
		accountByAuthID: map[string]*accountSpec{"protected.json": account},
	}}

	selected, err := selector.Pick(
		context.Background(),
		"codex",
		"gpt-5.4",
		cliproxyexecutor.Options{},
		[]*coreauth.Auth{{ID: "protected.json"}},
	)
	if err != nil {
		t.Fatalf("Pick: %v", err)
	}
	if selected == nil || selected.ID != "protected.json" {
		t.Fatalf("expected auth with explicitly absent hourly window, got %#v", selected)
	}
}

func TestCockpitSelectorPickFailsClosedForUnknownBoundOAuthQuota(t *testing.T) {
	hourlyThreshold := 10
	weeklyThreshold := 20
	snapshotUpdatedAt := time.Now().Unix()
	weeklyWindowPresent := false
	account := &accountSpec{
		ID:     "protected",
		Email:  "protected@example.com",
		AuthID: "protected.json",
		QuotaReserve: &quotaReserveSpec{
			HourlyThresholdPercent:       &hourlyThreshold,
			WeeklyThresholdPercent:       &weeklyThreshold,
			SnapshotUpdatedAtUnixSeconds: &snapshotUpdatedAt,
			HourlyRemainingPercent:       nil,
			WeeklyRemainingPercent:       nil,
			HourlyWindowPresent:          nil,
			WeeklyWindowPresent:          &weeklyWindowPresent,
		},
	}
	selector := &cockpitSelector{manifest: &manifest{
		accountByAuthID: map[string]*accountSpec{"protected.json": account},
	}}

	selected, err := selector.Pick(
		context.Background(),
		"codex",
		"gpt-5.4",
		cliproxyexecutor.Options{},
		[]*coreauth.Auth{{ID: "protected.json"}},
	)
	if selected != nil {
		t.Fatalf("expected no selected auth, got %#v", selected)
	}
	if err == nil {
		t.Fatal("expected quota reserve error")
	}
	message := err.Error()
	for _, fragment := range []string{
		"no auth available",
		"bound OAuth quota reserve blocked 1 auth(s)",
		"protected@example.com",
		"5h remaining quota unknown",
	} {
		if !strings.Contains(message, fragment) {
			t.Fatalf("expected %q in error %q", fragment, message)
		}
	}
}

func TestCockpitSelectorPickFailsClosedForInvalidQuotaSnapshotTimestamp(t *testing.T) {
	now := time.Now().Unix()
	tests := []struct {
		name      string
		timestamp *int64
		reason    string
	}{
		{name: "missing", timestamp: nil, reason: "quota snapshot timestamp unknown"},
		{name: "non-positive", timestamp: int64PointerForTest(0), reason: "quota snapshot timestamp invalid"},
		{name: "future", timestamp: int64PointerForTest(now + 60), reason: "quota snapshot timestamp invalid"},
		{name: "stale", timestamp: int64PointerForTest(now - int64(quotaReserveMaxSnapshotAge/time.Second) - 1), reason: "quota snapshot stale"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			hourlyThreshold := 10
			weeklyThreshold := 20
			hourlyRemaining := 80
			weeklyRemaining := 80
			windowPresent := true
			account := &accountSpec{
				ID:     "protected",
				Email:  "protected@example.com",
				AuthID: "protected.json",
				QuotaReserve: &quotaReserveSpec{
					HourlyThresholdPercent:       &hourlyThreshold,
					WeeklyThresholdPercent:       &weeklyThreshold,
					SnapshotUpdatedAtUnixSeconds: tt.timestamp,
					HourlyRemainingPercent:       &hourlyRemaining,
					WeeklyRemainingPercent:       &weeklyRemaining,
					HourlyWindowPresent:          &windowPresent,
					WeeklyWindowPresent:          &windowPresent,
				},
			}
			selector := &cockpitSelector{manifest: &manifest{
				accountByAuthID: map[string]*accountSpec{"protected.json": account},
			}}

			selected, err := selector.Pick(
				context.Background(),
				"codex",
				"gpt-5.4",
				cliproxyexecutor.Options{},
				[]*coreauth.Auth{{ID: "protected.json"}},
			)
			if selected != nil {
				t.Fatalf("expected no selected auth, got %#v", selected)
			}
			if err == nil || !strings.Contains(err.Error(), tt.reason) {
				t.Fatalf("expected %q in quota reserve error, got %v", tt.reason, err)
			}
		})
	}
}

func TestQuotaReserveStateStoreHotReloadsSnapshot(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "quota-reserve.json")
	hourlyThreshold := 20
	weeklyThreshold := 10
	account := &accountSpec{
		ID:    "protected",
		Email: "protected@example.com",
		QuotaReserve: &quotaReserveSpec{
			HourlyThresholdPercent: &hourlyThreshold,
			WeeklyThresholdPercent: &weeklyThreshold,
		},
	}
	writeState := func(hourly, weekly int) {
		t.Helper()
		content, err := json.Marshal(quotaReserveStateFile{Accounts: map[string]quotaReserveSnapshot{
			"protected": {
				SnapshotUpdatedAtUnixSeconds: int64PointerForTest(time.Now().Unix()),
				HourlyRemainingPercent:       intPointerForTest(hourly),
				WeeklyRemainingPercent:       intPointerForTest(weekly),
				HourlyWindowPresent:          boolPointerForTest(true),
				WeeklyWindowPresent:          boolPointerForTest(true),
			},
		}})
		if err != nil {
			t.Fatalf("marshal quota reserve state: %v", err)
		}
		if err := os.WriteFile(statePath, content, 0o600); err != nil {
			t.Fatalf("write quota reserve state: %v", err)
		}
	}

	writeState(80, 80)
	store := newQuotaReserveStateStore(statePath, nil)
	if err := store.load(); err != nil {
		t.Fatalf("load available state: %v", err)
	}
	if reason := quotaReserveBlockReasonWithState(account, store, time.Now()); reason != "" {
		t.Fatalf("expected available snapshot, got %q", reason)
	}

	writeState(20, 80)
	if err := store.load(); err != nil {
		t.Fatalf("load blocked state: %v", err)
	}
	if reason := quotaReserveBlockReasonWithState(account, store, time.Now()); !strings.Contains(reason, "5h remaining 20% <= reserve 20%") {
		t.Fatalf("expected hot-reloaded reserve block, got %q", reason)
	}
}

func TestQuotaReserveSelectorFiltersCachedSessionAffinityAuth(t *testing.T) {
	tests := []struct {
		name          string
		includeNormal bool
		mutateReserve func(*quotaReserveSpec)
		wantAuthID    string
		wantError     string
	}{
		{
			name:          "blocked reselects normal",
			includeNormal: true,
			mutateReserve: func(reserve *quotaReserveSpec) {
				*reserve.HourlyRemainingPercent = *reserve.HourlyThresholdPercent
			},
			wantAuthID: "normal.json",
		},
		{
			name:          "stale reselects normal",
			includeNormal: true,
			mutateReserve: func(reserve *quotaReserveSpec) {
				*reserve.SnapshotUpdatedAtUnixSeconds = time.Now().Add(-quotaReserveMaxSnapshotAge - time.Second).Unix()
			},
			wantAuthID: "normal.json",
		},
		{
			name: "blocked without fallback returns quota error",
			mutateReserve: func(reserve *quotaReserveSpec) {
				*reserve.WeeklyRemainingPercent = *reserve.WeeklyThresholdPercent
			},
			wantError: "bound OAuth quota reserve blocked 1 auth(s)",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			hourlyThreshold := 10
			weeklyThreshold := 20
			snapshotUpdatedAt := time.Now().Unix()
			hourlyRemaining := 80
			weeklyRemaining := 80
			windowPresent := true
			protectedPlanRank := 2
			normalPlanRank := 1
			reserve := &quotaReserveSpec{
				HourlyThresholdPercent:       &hourlyThreshold,
				WeeklyThresholdPercent:       &weeklyThreshold,
				SnapshotUpdatedAtUnixSeconds: &snapshotUpdatedAt,
				HourlyRemainingPercent:       &hourlyRemaining,
				WeeklyRemainingPercent:       &weeklyRemaining,
				HourlyWindowPresent:          &windowPresent,
				WeeklyWindowPresent:          &windowPresent,
			}
			protectedAccount := &accountSpec{
				ID:           "protected",
				Email:        "protected@example.com",
				AuthID:       "protected.json",
				PlanRank:     &protectedPlanRank,
				QuotaReserve: reserve,
			}
			normalAccount := &accountSpec{
				ID:       "normal",
				Email:    "normal@example.com",
				AuthID:   "normal.json",
				PlanRank: &normalPlanRank,
			}
			m := &manifest{
				Accounts:          []accountSpec{*protectedAccount, *normalAccount},
				RoutingStrategy:   "plan_high_first",
				accountByID:       map[string]*accountSpec{"protected": protectedAccount, "normal": normalAccount},
				accountByAuthID:   map[string]*accountSpec{"protected.json": protectedAccount, "normal.json": normalAccount},
				originalIndexByID: map[string]int{"protected": 0, "normal": 1},
			}
			cfg := &config.Config{}
			cfg.Routing.SessionAffinity = true
			cfg.Routing.SessionAffinityTTL = time.Minute.String()
			selector := buildCoreAuthSelector(cfg, &cockpitSelector{manifest: m}, m, nil)
			if stoppable, ok := selector.(coreauth.StoppableSelector); ok {
				defer stoppable.Stop()
			}

			auths := []*coreauth.Auth{{ID: "protected.json"}}
			if tt.includeNormal {
				auths = append(auths, &coreauth.Auth{ID: "normal.json"})
			}
			opts := cliproxyexecutor.Options{
				OriginalRequest: []byte(`{"metadata":{"user_id":"user_xxx_account__session_ac980658-63bd-4fb3-97ba-8da64cb1e344"}}`),
			}

			first, err := selector.Pick(context.Background(), "codex", "gpt-5.4", opts, auths)
			if err != nil {
				t.Fatalf("initial Pick: %v", err)
			}
			if first == nil || first.ID != "protected.json" {
				t.Fatalf("expected protected auth to establish affinity, got %#v", first)
			}
			cached, err := selector.Pick(context.Background(), "codex", "gpt-5.4", opts, auths)
			if err != nil || cached == nil || cached.ID != "protected.json" {
				t.Fatalf("expected protected affinity cache hit, got auth=%#v err=%v", cached, err)
			}

			tt.mutateReserve(reserve)
			selected, err := selector.Pick(context.Background(), "codex", "gpt-5.4", opts, auths)
			if tt.wantError != "" {
				if selected != nil {
					t.Fatalf("expected no auth after reserve block, got %#v", selected)
				}
				if err == nil || !strings.Contains(err.Error(), tt.wantError) {
					t.Fatalf("expected quota error containing %q, got %v", tt.wantError, err)
				}
				return
			}
			if err != nil {
				t.Fatalf("Pick after reserve change: %v", err)
			}
			if selected == nil || selected.ID != tt.wantAuthID {
				t.Fatalf("expected %s after cached auth was filtered, got %#v", tt.wantAuthID, selected)
			}
		})
	}
}

func TestBackupAccountSelectorOverridesCachedAffinityWhenRegularRecovers(t *testing.T) {
	regularAccount := &accountSpec{ID: "regular", AuthID: "regular.json"}
	backupAccount := &accountSpec{ID: "backup", AuthID: "backup.json"}
	m := &manifest{
		Accounts:        []accountSpec{*regularAccount, *backupAccount},
		RoutingStrategy: "custom",
		CustomRoutingRules: []customRoutingRule{
			{AccountID: "regular", Priority: 0, Weight: 1},
			{AccountID: "backup", Priority: 100, Weight: 1, IsBackup: true},
		},
		accountByID: map[string]*accountSpec{
			"regular": regularAccount,
			"backup":  backupAccount,
		},
		accountByAuthID: map[string]*accountSpec{
			"regular.json": regularAccount,
			"backup.json":  backupAccount,
		},
		originalIndexByID: map[string]int{"regular": 0, "backup": 1},
	}
	cfg := &config.Config{}
	cfg.Routing.SessionAffinity = true
	cfg.Routing.SessionAffinityTTL = time.Minute.String()
	selector := buildCoreAuthSelector(cfg, &cockpitSelector{manifest: m}, m, nil)
	if stoppable, ok := selector.(coreauth.StoppableSelector); ok {
		defer stoppable.Stop()
	}

	regularAuth := &coreauth.Auth{
		ID:             "regular.json",
		Unavailable:    true,
		NextRetryAfter: time.Now().Add(time.Minute),
	}
	backupAuth := &coreauth.Auth{ID: "backup.json"}
	auths := []*coreauth.Auth{regularAuth, backupAuth}
	opts := cliproxyexecutor.Options{
		OriginalRequest: []byte(`{"metadata":{"user_id":"user_xxx_account__session_43d54db9-d7ba-4b2f-b09a-47f238dc78ac"}}`),
	}

	selected, err := selector.Pick(context.Background(), "codex", "gpt-5.4", opts, auths)
	if err != nil || selected == nil || selected.ID != "backup.json" {
		t.Fatalf("expected backup while regular is unavailable, got auth=%#v err=%v", selected, err)
	}

	regularAuth.Unavailable = false
	regularAuth.NextRetryAfter = time.Time{}
	selected, err = selector.Pick(context.Background(), "codex", "gpt-5.4", opts, auths)
	if err != nil || selected == nil || selected.ID != "regular.json" {
		t.Fatalf("expected recovered regular auth to override backup affinity, got auth=%#v err=%v", selected, err)
	}
}

func TestUsagePrioritySelectorPrefersHighestAcrossRoutingStrategies(t *testing.T) {
	preferredAccount := &accountSpec{ID: "preferred", AuthID: "preferred.json"}
	regularAccount := &accountSpec{ID: "regular", AuthID: "regular.json"}
	backupAccount := &accountSpec{ID: "backup", AuthID: "backup.json"}
	m := &manifest{
		Accounts:        []accountSpec{*preferredAccount, *regularAccount, *backupAccount},
		RoutingStrategy: "auto",
		CustomRoutingRules: []customRoutingRule{
			{AccountID: "preferred", IsPreferred: true},
			{AccountID: "regular"},
			{AccountID: "backup", IsBackup: true},
		},
		accountByID: map[string]*accountSpec{
			"preferred": preferredAccount,
			"regular":   regularAccount,
			"backup":    backupAccount,
		},
		accountByAuthID: map[string]*accountSpec{
			"preferred.json": preferredAccount,
			"regular.json":   regularAccount,
			"backup.json":    backupAccount,
		},
		originalIndexByID: map[string]int{"preferred": 0, "regular": 1, "backup": 2},
	}
	cfg := &config.Config{}
	cfg.Routing.SessionAffinity = true
	cfg.Routing.SessionAffinityTTL = time.Minute.String()
	selector := buildCoreAuthSelector(cfg, &cockpitSelector{manifest: m}, m, nil)
	if stoppable, ok := selector.(coreauth.StoppableSelector); ok {
		defer stoppable.Stop()
	}

	preferredAuth := &coreauth.Auth{
		ID:             "preferred.json",
		Unavailable:    true,
		NextRetryAfter: time.Now().Add(time.Minute),
	}
	regularAuth := &coreauth.Auth{ID: "regular.json"}
	backupAuth := &coreauth.Auth{ID: "backup.json"}
	auths := []*coreauth.Auth{preferredAuth, regularAuth, backupAuth}
	opts := cliproxyexecutor.Options{
		OriginalRequest: []byte(`{"metadata":{"user_id":"user_xxx_account__session_c425b37d-e64d-4798-aef9-c8b0402fd713"}}`),
	}

	selected, err := selector.Pick(context.Background(), "codex", "gpt-5.4", opts, auths)
	if err != nil || selected == nil || selected.ID != "regular.json" {
		t.Fatalf("expected regular while preferred is unavailable, got auth=%#v err=%v", selected, err)
	}

	preferredAuth.Unavailable = false
	preferredAuth.NextRetryAfter = time.Time{}
	selected, err = selector.Pick(context.Background(), "codex", "gpt-5.4", opts, auths)
	if err != nil || selected == nil || selected.ID != "preferred.json" {
		t.Fatalf("expected recovered preferred auth to override regular affinity, got auth=%#v err=%v", selected, err)
	}

	preferredAuth.Unavailable = true
	preferredAuth.NextRetryAfter = time.Now().Add(time.Minute)
	regularAuth.Unavailable = true
	regularAuth.NextRetryAfter = time.Now().Add(time.Minute)
	selected, err = selector.Pick(context.Background(), "codex", "gpt-5.4", opts, auths)
	if err != nil || selected == nil || selected.ID != "backup.json" {
		t.Fatalf("expected backup when preferred and regular are unavailable, got auth=%#v err=%v", selected, err)
	}
}

func int64PointerForTest(value int64) *int64 {
	return &value
}

func intPointerForTest(value int) *int {
	return &value
}

func boolPointerForTest(value bool) *bool {
	return &value
}

func TestSidecarRuntimeRegistersConfigCodexAPIKeyAuths(t *testing.T) {
	tempDir := t.TempDir()
	authDir := filepath.Join(tempDir, "auths")
	configPath := filepath.Join(tempDir, "config.json")
	if err := os.WriteFile(configPath, []byte(`{}`), 0o644); err != nil {
		t.Fatalf("write config path: %v", err)
	}

	cfg := &config.Config{
		AuthDir: authDir,
		CodexKey: []config.CodexKey{{
			APIKey:  "sk-upstream",
			BaseURL: "http://127.0.0.1:1",
		}},
	}
	account := &accountSpec{ID: "api-account", Email: "api@example.com", UpstreamAPIKey: "sk-upstream"}
	m := &manifest{
		Accounts:        []accountSpec{*account},
		accountByID:     map[string]*accountSpec{"api-account": account},
		accountByAuthID: map[string]*accountSpec{},
		accountByAPIKey: map[string]*accountSpec{"sk-upstream": account},
		ModelIDs:        []string{"gpt-5.4"},
	}
	manager := buildCoreAuthManager(cfg, &cockpitSelector{manifest: m}, &authHook{manifest: m}, m, nil, newRequestUsageTracker())

	runtime, err := newSidecarRuntime(context.Background(), configPath, cfg, m, manager)
	if err != nil {
		t.Fatalf("newSidecarRuntime: %v", err)
	}
	defer runtime.Stop()

	var codexAPIKeyAuth *coreauth.Auth
	for _, auth := range manager.List() {
		if auth == nil || !strings.EqualFold(auth.Provider, "codex") {
			continue
		}
		if auth.Attributes != nil && strings.TrimSpace(auth.Attributes["api_key"]) == "sk-upstream" {
			codexAPIKeyAuth = auth
			break
		}
	}
	if codexAPIKeyAuth == nil {
		t.Fatalf("expected codex API Key auth to be registered, got %#v", manager.List())
	}
	if got := m.accountByAuthID[strings.ToLower(codexAPIKeyAuth.ID)]; got == nil || got.ID != "api-account" {
		t.Fatalf("expected auth to be linked to manifest account, got %#v", got)
	}
}

func TestSidecarRuntimeRegistersManifestCodexAccessTokenAuths(t *testing.T) {
	tempDir := t.TempDir()
	authDir := filepath.Join(tempDir, "auths")
	if err := os.MkdirAll(authDir, 0o755); err != nil {
		t.Fatalf("create auth dir: %v", err)
	}
	configPath := filepath.Join(tempDir, "config.json")
	if err := os.WriteFile(configPath, []byte(`{}`), 0o644); err != nil {
		t.Fatalf("write config path: %v", err)
	}
	authFile := filepath.Join(authDir, "token-account.json")
	if err := os.WriteFile(authFile, []byte(`{
		"type":"codex",
		"email":"token@example.com",
		"access_token":"session-runtime-token",
		"personal_access_token":"at-runtime-token",
		"at_token":"at-runtime-token",
		"account_id":"acct-token",
		"openai_auth_mode":"personal_access_token",
		"proxy_url":"http://127.0.0.1:9"
	}`), 0o600); err != nil {
		t.Fatalf("write auth file: %v", err)
	}

	cfg := &config.Config{AuthDir: authDir}
	account := &accountSpec{
		ID:               "token-account",
		Email:            "token@example.com",
		AuthID:           "token-account.json",
		AuthKind:         "access_token",
		AccessTokenOnly:  true,
		ChatGPTAccountID: "acct-token",
	}
	m := &manifest{
		Accounts:         []accountSpec{*account},
		accountByID:      map[string]*accountSpec{"token-account": account},
		accountByAuthID:  map[string]*accountSpec{"token-account.json": account},
		accountByAPIKey:  map[string]*accountSpec{},
		accountByChatGPT: map[string]*accountSpec{"acct-token": account},
		accountByEmail:   map[string]*accountSpec{"token@example.com": account},
		ModelIDs:         []string{"gpt-5.4"},
	}
	manager := buildCoreAuthManager(cfg, &cockpitSelector{manifest: m}, &authHook{manifest: m}, m, nil, newRequestUsageTracker())

	runtime, err := newSidecarRuntime(context.Background(), configPath, cfg, m, manager)
	if err != nil {
		t.Fatalf("newSidecarRuntime: %v", err)
	}
	defer runtime.Stop()

	var tokenAuth *coreauth.Auth
	for _, auth := range manager.List() {
		if auth == nil || !strings.EqualFold(auth.Provider, "codex") {
			continue
		}
		if auth.Metadata != nil && auth.Metadata["access_token"] == "at-runtime-token" {
			tokenAuth = auth
			break
		}
	}
	if tokenAuth == nil {
		t.Fatalf("expected codex access token auth to be registered, got %#v", manager.List())
	}
	if tokenAuth.ProxyURL != "http://127.0.0.1:9" {
		t.Fatalf("expected proxy url from auth metadata, got %q", tokenAuth.ProxyURL)
	}
	if got := m.accountByAuthID[strings.ToLower(tokenAuth.ID)]; got == nil || got.ID != "token-account" {
		t.Fatalf("expected token auth to be linked to manifest account, got %#v", got)
	}
	if info := findModelInfoForTest(
		registry.GetGlobalRegistry().GetModelsForClient(tokenAuth.ID),
		"gpt-5.4",
	); info == nil {
		t.Fatalf("expected manifest models to be registered for token auth")
	}
}

func TestManifestRegistryModelsPreservesStaticThinkingSupport(t *testing.T) {
	models := manifestRegistryModels(&manifest{
		ModelIDs: []string{"gpt-5.2"},
	})

	info := findModelInfoForTest(models, "gpt-5.2")
	if info == nil {
		t.Fatalf("expected gpt-5.2 in manifest registry models: %#v", models)
	}
	if info.Thinking == nil {
		t.Fatalf("expected gpt-5.2 to preserve static thinking support: %#v", info)
	}
	if !stringSliceContains(info.Thinking.Levels, "high") {
		t.Fatalf("expected gpt-5.2 thinking levels to include high: %#v", info.Thinking.Levels)
	}
	if info.UserDefined {
		t.Fatalf("static model should not be marked user-defined: %#v", info)
	}
}

func TestManifestRegistryModelsCopiesSourceThinkingToAliases(t *testing.T) {
	models := manifestRegistryModels(&manifest{
		ModelAliases: []modelAliasSpec{{
			SourceModel: "gpt-5.2",
			Alias:       "gpt-5.2-codex",
			Fork:        true,
		}},
	})

	alias := findModelInfoForTest(models, "gpt-5.2-codex")
	if alias == nil {
		t.Fatalf("expected alias in manifest registry models: %#v", models)
	}
	if alias.Thinking == nil {
		t.Fatalf("expected alias to inherit source thinking support: %#v", alias)
	}
	if !stringSliceContains(alias.Thinking.Levels, "high") {
		t.Fatalf("expected alias thinking levels to include high: %#v", alias.Thinking.Levels)
	}
	if alias.UserDefined {
		t.Fatalf("alias backed by static source should not be marked user-defined: %#v", alias)
	}
}

func TestManifestRegistryModelsTreatsUnknownModelsAsUserDefined(t *testing.T) {
	models := manifestRegistryModels(&manifest{
		ModelIDs: []string{"custom-codex-model"},
	})

	info := findModelInfoForTest(models, "custom-codex-model")
	if info == nil {
		t.Fatalf("expected custom model in manifest registry models: %#v", models)
	}
	if !info.UserDefined {
		t.Fatalf("unknown manifest model should be user-defined so thinking passes upstream: %#v", info)
	}
	if info.Thinking != nil {
		t.Fatalf("unknown manifest model should not invent thinking support: %#v", info)
	}
}

func TestManifestRegisteredModelsPreserveReasoningEffortThroughThinkingPipeline(t *testing.T) {
	auth := &coreauth.Auth{
		ID:       "test-codex-auth",
		Provider: "codex",
		Status:   coreauth.StatusActive,
	}
	manager := buildCoreAuthManager(&config.Config{}, &cockpitSelector{}, nil, nil, nil, nil)
	registered, err := manager.Register(context.Background(), auth)
	if err != nil {
		t.Fatalf("register auth: %v", err)
	}
	auth = registered
	t.Cleanup(func() {
		registry.GetGlobalRegistry().UnregisterClient(auth.ID)
	})

	registerManifestModelsForAuth(manager, &manifest{
		ModelIDs: []string{"gpt-5.2"},
		ModelAliases: []modelAliasSpec{{
			SourceModel: "gpt-5.2",
			Alias:       "gpt-5.2-codex",
		}},
	}, auth)

	for _, model := range []string{"gpt-5.2", "gpt-5.2-codex"} {
		out, err := thinking.ApplyThinking(
			[]byte(`{"model":"`+model+`","reasoning":{"effort":"high"}}`),
			model,
			"openai-response",
			"codex",
			"codex",
		)
		if err != nil {
			t.Fatalf("ApplyThinking(%s): %v", model, err)
		}
		var payload map[string]any
		if err := json.Unmarshal(out, &payload); err != nil {
			t.Fatalf("translated payload for %s should be JSON: %v", model, err)
		}
		reasoning, _ := payload["reasoning"].(map[string]any)
		if reasoning["effort"] != "high" {
			t.Fatalf("reasoning effort should survive manifest registry for %s: %s", model, out)
		}
	}
}

func findModelInfoForTest(models []*cliproxy.ModelInfo, id string) *cliproxy.ModelInfo {
	for _, model := range models {
		if model != nil && strings.EqualFold(model.ID, id) {
			return model
		}
	}
	return nil
}

func stringSliceContains(values []string, target string) bool {
	for _, value := range values {
		if strings.EqualFold(value, target) {
			return true
		}
	}
	return false
}

func TestBuiltinTranslatorNormalizesOpenAIResponsesForCodex(t *testing.T) {
	in := []byte(`{"model":"gpt-5.4-mini","input":"pong","stream":false,"temperature":0.1}`)
	out := sdktranslator.TranslateRequest(
		sdktranslator.FormatOpenAIResponse,
		sdktranslator.FormatCodex,
		"gpt-5.4-mini",
		in,
		true,
	)

	var payload map[string]any
	if err := json.Unmarshal(out, &payload); err != nil {
		t.Fatalf("translated payload should be JSON: %v", err)
	}
	if payload["stream"] != true {
		t.Fatalf("stream should be forced true, got %#v", payload["stream"])
	}
	if _, exists := payload["temperature"]; exists {
		t.Fatalf("unsupported temperature leaked into Codex payload: %s", out)
	}
	input, ok := payload["input"].([]any)
	if !ok || len(input) != 1 {
		t.Fatalf("input should be normalized to a message list, got %#v", payload["input"])
	}
	first, ok := input[0].(map[string]any)
	if !ok || first["type"] != "message" || first["role"] != "user" {
		t.Fatalf("unexpected normalized input item: %#v", input[0])
	}
}

func TestRequestPolicyMiddlewareSetsCPAUsageAPIKey(t *testing.T) {
	gin.SetMode(gin.TestMode)
	m := &manifest{
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "key_1", Label: "Test key", Key: "client-key", Enabled: true},
		},
	}
	policy := &requestPolicy{manifest: m}
	router := gin.New()
	router.Use(policy.middleware())
	router.GET("/v1/responses", func(c *gin.Context) {
		value, exists := c.Get(ginUserAPIKeyKey)
		if !exists {
			t.Fatalf("%s should be set for CPA usage reporter", ginUserAPIKeyKey)
		}
		if value != "client-key" {
			t.Fatalf("unexpected %s: %#v", ginUserAPIKeyKey, value)
		}
		c.Status(http.StatusNoContent)
	})

	req := httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
	req.Header.Set("Authorization", "Bearer client-key")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusNoContent {
		t.Fatalf("unexpected status: %d", w.Code)
	}
}

func TestAPIKeyTokenLimiterAccumulatesAcrossModels(t *testing.T) {
	m := &manifest{APIKeys: []apiKeySpec{{
		ID:         "key_1",
		Key:        "client-key",
		TokenLimit: 10_000_000,
		TokenUsed:  9_000_000,
		Enabled:    true,
	}}}
	limiter := newAPIKeyTokenLimiter(m)
	spec := &m.APIKeys[0]
	if used, limit, exceeded := limiter.exceeded(spec); used != 9_000_000 || limit != 10_000_000 || exceeded {
		t.Fatalf("unexpected initial limiter state: used=%d limit=%d exceeded=%v", used, limit, exceeded)
	}
	limiter.addUsage(spec, 400_000)
	limiter.addUsage(spec, 600_000)
	if used, limit, exceeded := limiter.exceeded(spec); used != 10_000_000 || limit != 10_000_000 || !exceeded {
		t.Fatalf("expected limit to be reached across requests: used=%d limit=%d exceeded=%v", used, limit, exceeded)
	}
}

func TestRequestPolicyBlocksKeyAtTokenLimit(t *testing.T) {
	gin.SetMode(gin.TestMode)
	m := &manifest{APIKeys: []apiKeySpec{{
		ID:         "key_1",
		Label:      "Limited key",
		Key:        "client-key",
		TokenLimit: 10_000_000,
		TokenUsed:  10_000_000,
		Enabled:    true,
	}}}
	m.apiKeyByValue = map[string]*apiKeySpec{"client-key": &m.APIKeys[0]}
	policy := &requestPolicy{
		manifest:     m,
		tracker:      newRequestUsageTracker(),
		tokenLimiter: newAPIKeyTokenLimiter(m),
	}
	router := gin.New()
	router.Use(policy.middleware())
	router.POST("/v1/responses", func(c *gin.Context) {
		t.Fatal("limited request should not reach the handler")
	})

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.5"}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusTooManyRequests {
		t.Fatalf("expected 429, got %d: %s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), `"code":"token_limit_exceeded"`) {
		t.Fatalf("expected token_limit_exceeded response: %s", w.Body.String())
	}
}

type testExecutorStatusError struct {
	status int
}

func (e testExecutorStatusError) Error() string {
	return http.StatusText(e.status)
}

func (e testExecutorStatusError) StatusCode() int {
	return e.status
}

func TestWriteExecutorErrorThrottlesRetryableDownstreamError(t *testing.T) {
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", nil)
	server := &relayServer{
		cfg: &config.Config{
			SDKConfig: config.SDKConfig{
				Streaming: config.StreamingConfig{
					BootstrapRetryBaseDelayMS: 50,
					BootstrapRetryMaxDelayMS:  50,
				},
			},
		},
	}

	started := time.Now()
	server.writeExecutorError(c, testExecutorStatusError{status: http.StatusServiceUnavailable})
	elapsed := time.Since(started)

	if elapsed < 50*time.Millisecond {
		t.Fatalf("expected downstream error delay >= 50ms, got %v", elapsed)
	}
	if recorder.Code != http.StatusServiceUnavailable {
		t.Fatalf("unexpected status: %d", recorder.Code)
	}
}

func TestRequestUsageTrackerFinalizesWithLastSuccessfulAttempt(t *testing.T) {
	tracker := newRequestUsageTracker()
	tracker.recordSelectedAccount("req-1", &accountSpec{
		ID:    "account-ok",
		Email: "ok@example.com",
	}, "auth-ok")
	tracker.record(usagePayload{
		Type:          "usage",
		RequestID:     "req-1",
		AccountID:     "account-failed",
		AccountEmail:  "failed@example.com",
		Model:         "gpt-5.5",
		RequestKind:   "text",
		Success:       false,
		Status:        http.StatusInternalServerError,
		ErrorCategory: "upstream_error",
		ErrorMessage:  "unexpected EOF",
	})
	tracker.record(usagePayload{
		Type:         "usage",
		RequestID:    "req-1",
		AccountID:    "account-ok",
		AccountEmail: "ok@example.com",
		Model:        "gpt-5.5",
		RequestKind:  "text",
		ServiceTier:  "priority",
		Success:      true,
		Status:       http.StatusOK,
		Usage: usageDetails{
			InputTokens:  10,
			OutputTokens: 5,
			TotalTokens:  15,
		},
	})

	payload, ok := tracker.finalize("req-1", usageFinalizeInput{
		spec:          &apiKeySpec{ID: "key_1", Label: "Default"},
		requestKind:   "text",
		model:         "gpt-5.5",
		status:        http.StatusOK,
		latencyMS:     446_000,
		completedAtMS: 123,
	})

	if !ok {
		t.Fatal("expected finalized usage payload")
	}
	if !payload.Success || payload.AccountID != "account-ok" {
		t.Fatalf("expected successful account payload, got %#v", payload)
	}
	if payload.ErrorCategory != "" || payload.ErrorMessage != "" {
		t.Fatalf("successful final request should not keep attempt error: %#v", payload)
	}
	if payload.LatencyMS != 446_000 || payload.APIKeyID != "key_1" {
		t.Fatalf("final request metadata was not applied: %#v", payload)
	}
	if payload.ServiceTier != "priority" {
		t.Fatalf("expected service tier to be preserved, got %#v", payload)
	}
}

func TestRequestUsageTrackerFinalizesWithSelectedAccount(t *testing.T) {
	tracker := newRequestUsageTracker()
	tracker.recordSelectedAccount("req-selected", &accountSpec{
		ID:    "account-selected",
		Email: "selected@example.com",
	}, "auth-selected")

	payload, ok := tracker.finalize("req-selected", usageFinalizeInput{
		spec:          &apiKeySpec{ID: "key_1", Label: "Default"},
		requestKind:   "text",
		model:         "gpt-5.5",
		status:        http.StatusOK,
		latencyMS:     100,
		completedAtMS: 123,
	})

	if !ok {
		t.Fatal("expected finalized usage payload")
	}
	if payload.AccountID != "account-selected" || payload.AccountEmail != "selected@example.com" || payload.AuthID != "auth-selected" {
		t.Fatalf("expected selected account metadata, got %#v", payload)
	}
}

func TestRequestUsageTrackerSelectedAccountOverridesUsageAccount(t *testing.T) {
	tracker := newRequestUsageTracker()
	tracker.recordSelectedAccount("req-usage", &accountSpec{
		ID:    "account-selected",
		Email: "selected@example.com",
	}, "auth-selected")
	tracker.record(usagePayload{
		Type:         "usage",
		RequestID:    "req-usage",
		AccountID:    "account-usage",
		AccountEmail: "usage@example.com",
		AuthID:       "auth-usage",
		Success:      true,
	})

	payload, ok := tracker.finalize("req-usage", usageFinalizeInput{
		status:        http.StatusOK,
		latencyMS:     100,
		completedAtMS: 123,
	})

	if !ok {
		t.Fatal("expected finalized usage payload")
	}
	if payload.AccountID != "account-selected" || payload.AccountEmail != "selected@example.com" || payload.AuthID != "auth-selected" {
		t.Fatalf("selected account metadata should win, got %#v", payload)
	}
}

type countingSelector struct {
	auth  *coreauth.Auth
	count int
}

func (s *countingSelector) Pick(context.Context, string, string, cliproxyexecutor.Options, []*coreauth.Auth) (*coreauth.Auth, error) {
	s.count++
	return s.auth, nil
}

func TestRecordingSelectorRecordsSessionAffinityCacheHit(t *testing.T) {
	account := &accountSpec{ID: "account-selected", Email: "selected@example.com"}
	m := &manifest{
		accountByAuthID: map[string]*accountSpec{"auth-selected": account},
		accountByID:     map[string]*accountSpec{"account-selected": account},
		accountByAPIKey: map[string]*accountSpec{},
	}
	auth := &coreauth.Auth{ID: "auth-selected", Provider: "codex", Status: coreauth.StatusActive}
	fallback := &countingSelector{auth: auth}
	affinity := coreauth.NewSessionAffinitySelectorWithConfig(coreauth.SessionAffinityConfig{
		Fallback: fallback,
		TTL:      time.Hour,
	})
	tracker := newRequestUsageTracker()
	selector := &recordingSelector{inner: affinity, manifest: m, tracker: tracker}
	headers := make(http.Header)
	headers.Set("X-Session-ID", "session-selected")
	opts := cliproxyexecutor.Options{Headers: headers}

	ctx1 := internallogging.WithRequestID(context.Background(), "req-first")
	if _, err := selector.Pick(ctx1, "codex", "gpt-5.5", opts, []*coreauth.Auth{auth}); err != nil {
		t.Fatalf("first pick: %v", err)
	}
	ctx2 := internallogging.WithRequestID(context.Background(), "req-cache")
	if _, err := selector.Pick(ctx2, "codex", "gpt-5.5", opts, []*coreauth.Auth{auth}); err != nil {
		t.Fatalf("cache pick: %v", err)
	}
	if fallback.count != 1 {
		t.Fatalf("expected second pick to use affinity cache, fallback count=%d", fallback.count)
	}

	payload, ok := tracker.finalize("req-cache", usageFinalizeInput{
		status:        http.StatusOK,
		latencyMS:     100,
		completedAtMS: 123,
	})
	if !ok {
		t.Fatal("expected finalized usage payload")
	}
	if payload.AccountID != "account-selected" || payload.AccountEmail != "selected@example.com" || payload.AuthID != "auth-selected" {
		t.Fatalf("expected cache hit selected account metadata, got %#v", payload)
	}
}

func TestRequestUsageTrackerKeepsStreamFailureAfterHTTPHeaders(t *testing.T) {
	tracker := newRequestUsageTracker()
	tracker.record(usagePayload{
		Type:          "usage",
		RequestID:     "req-2",
		AccountID:     "account-failed",
		Model:         "gpt-5.5",
		RequestKind:   "text",
		Success:       false,
		ErrorCategory: "request_failed",
		ErrorMessage:  "stream closed",
	})

	payload, ok := tracker.finalize("req-2", usageFinalizeInput{
		requestKind:   "text",
		model:         "gpt-5.5",
		status:        http.StatusOK,
		latencyMS:     100,
		completedAtMS: 123,
	})

	if !ok {
		t.Fatal("expected finalized usage payload")
	}
	if payload.Success || payload.ErrorCategory != "request_failed" {
		t.Fatalf("stream failure should remain failed even when HTTP status is 200: %#v", payload)
	}
}

func TestRequestPolicyEmitsRequestDiagnostics(t *testing.T) {
	gin.SetMode(gin.TestMode)
	m := &manifest{
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "key_1", Label: "Test key", Key: "client-key", Enabled: true},
		},
	}
	policy := &requestPolicy{manifest: m, emitter: &eventEmitter{}}
	router := gin.New()
	router.Use(policy.middleware())
	router.GET("/v1/responses", func(c *gin.Context) {
		if internallogging.GetRequestID(c.Request.Context()) == "" {
			t.Fatalf("request id should be attached to request context")
		}
		c.Status(http.StatusNoContent)
	})

	out := captureStdout(t, func() {
		req := httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
		req.Header.Set("Authorization", "Bearer client-key")
		router.ServeHTTP(httptest.NewRecorder(), req)
	})
	lines := strings.Split(strings.TrimSpace(out), "\n")
	if len(lines) != 2 {
		t.Fatalf("expected start and complete diagnostics, got %d lines:\n%s", len(lines), out)
	}
	var start requestDiagnosticPayload
	if err := json.Unmarshal([]byte(lines[0]), &start); err != nil {
		t.Fatalf("start diagnostic should be JSON: %v\n%s", err, lines[0])
	}
	var complete requestDiagnosticPayload
	if err := json.Unmarshal([]byte(lines[1]), &complete); err != nil {
		t.Fatalf("complete diagnostic should be JSON: %v\n%s", err, lines[1])
	}
	if start.Type != "request_started" || complete.Type != "request_completed" {
		t.Fatalf("unexpected diagnostic types: %#v %#v", start.Type, complete.Type)
	}
	if start.RequestID == "" || complete.RequestID != start.RequestID {
		t.Fatalf("request id should be stable across diagnostics: %#v %#v", start, complete)
	}
	if complete.Status != http.StatusNoContent || complete.RequestKind != "text" || complete.APIKeyID != "key_1" {
		t.Fatalf("unexpected completion diagnostic: %#v", complete)
	}
}

func TestUsagePluginResolvesAPIKeyAndRequestKindFromCPARecord(t *testing.T) {
	m := &manifest{
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "key_1", Label: "Test key", Key: "client-key", Enabled: true},
		},
	}
	tracker := newRequestUsageTracker()
	plugin := &usagePlugin{manifest: m, tracker: tracker}
	ctx := internallogging.WithRequestID(context.Background(), "req-1")
	ctx = internallogging.WithEndpoint(ctx, "POST /v1/responses")

	plugin.HandleUsage(ctx, coreusage.Record{
		Provider:    "codex",
		Model:       "gpt-5.4-mini",
		APIKey:      "client-key",
		RequestedAt: time.UnixMilli(123),
		Latency:     50 * time.Millisecond,
	})

	payload, ok := tracker.finalize("req-1", usageFinalizeInput{
		status:        http.StatusOK,
		latencyMS:     50,
		completedAtMS: 123,
	})
	if !ok {
		t.Fatal("expected usage payload")
	}
	if payload.APIKeyID != "key_1" || payload.APIKeyLabel != "Test key" {
		t.Fatalf("API key metadata was not resolved: %#v", payload)
	}
	if payload.RequestID != "req-1" {
		t.Fatalf("request id should be forwarded, got %q", payload.RequestID)
	}
	if payload.RequestKind != "text" {
		t.Fatalf("request kind should be inferred from endpoint, got %q", payload.RequestKind)
	}
}

func TestUsagePluginForwardsReasoningEffortInUsagePayload(t *testing.T) {
	tracker := newRequestUsageTracker()
	plugin := &usagePlugin{tracker: tracker}
	ctx := internallogging.WithRequestID(context.Background(), "req-reasoning")
	ctx = internallogging.WithEndpoint(ctx, "POST /v1/responses")

	plugin.HandleUsage(ctx, coreusage.Record{
		Provider:        "codex",
		Model:           "gpt-5.4",
		ReasoningEffort: "xhigh",
		RequestedAt:     time.UnixMilli(123),
		Latency:         50 * time.Millisecond,
	})

	payload, ok := tracker.finalize("req-reasoning", usageFinalizeInput{
		status:        http.StatusOK,
		latencyMS:     50,
		completedAtMS: 123,
	})
	if !ok {
		t.Fatal("expected usage payload")
	}
	if payload.ReasoningEffort != "xhigh" {
		t.Fatalf("reasoning effort was not forwarded: %#v", payload)
	}

	wire, err := json.Marshal(payload)
	if err != nil {
		t.Fatalf("marshal usage payload: %v", err)
	}
	var decoded map[string]any
	if err := json.Unmarshal(wire, &decoded); err != nil {
		t.Fatalf("decode usage payload: %v", err)
	}
	if got, ok := decoded["reasoningEffort"].(string); !ok || got != "xhigh" {
		t.Fatalf("usage JSON reasoningEffort = %#v, want xhigh", decoded["reasoningEffort"])
	}
}

func TestErrorCategoryClassifiesClientCanceled(t *testing.T) {
	if got := errorCategory(0, "context canceled", false); got != "client_canceled" {
		t.Fatalf("expected client_canceled, got %q", got)
	}
	if got := errorCategory(http.StatusGatewayTimeout, `Post "https://chatgpt.com/backend-api/codex/responses": context canceled`, false); got != "gateway_context_canceled" {
		t.Fatalf("expected gateway_context_canceled for upstream context cancellation, got %q", got)
	}
	if got := errorCategory(http.StatusBadGateway, "write tcp: broken pipe", false); got != "client_canceled" {
		t.Fatalf("expected client_canceled for broken pipe, got %q", got)
	}
	if got := errorCategory(http.StatusGatewayTimeout, "upstream timed out in stream_open attempt=1/1 after 60s", false); got != "upstream_first_byte_timeout" {
		t.Fatalf("expected upstream_first_byte_timeout, got %q", got)
	}
}

func TestAuthHookEmitsRequestScopedResultDiagnostics(t *testing.T) {
	apiKey := &apiKeySpec{ID: "key_1", Label: "Test key", Key: "client-key", Enabled: true}
	account := &accountSpec{ID: "account_1", Email: "user@example.com", AuthID: "auth.json"}
	m := &manifest{
		accountByAuthID: map[string]*accountSpec{"auth.json": account},
		accountByID:     map[string]*accountSpec{"auth": account},
	}
	hook := &authHook{manifest: m, emitter: &eventEmitter{}}
	ctx := internallogging.WithRequestID(context.Background(), "req-2")
	ctx = context.WithValue(ctx, clientAPIKeyContextKey, apiKey)
	ctx = context.WithValue(ctx, requestKindContextKey, "text")
	ctx = context.WithValue(ctx, requestModelContextKey, "gpt-5.5")

	out := captureStdout(t, func() {
		hook.OnResult(ctx, coreauth.Result{
			AuthID:          "auth.json",
			Provider:        "codex",
			Model:           "upstream-model",
			Success:         false,
			AuthStateKnown:  true,
			AuthAvailable:   false,
			NextRetryAt:     time.Now().Add(30 * time.Minute),
			AuthStateReason: "unauthorized",
			Error: &coreauth.Error{
				Code:       "upstream_timeout",
				Message:    "upstream timed out",
				Retryable:  true,
				HTTPStatus: http.StatusGatewayTimeout,
			},
		})
	})

	var payload requestDiagnosticPayload
	if err := json.Unmarshal([]byte(out), &payload); err != nil {
		t.Fatalf("auth result diagnostic should be JSON: %v\n%s", err, out)
	}
	if payload.Type != "auth_result" || payload.RequestID != "req-2" {
		t.Fatalf("unexpected auth result diagnostic identity: %#v", payload)
	}
	if payload.Model != "upstream-model" || payload.AccountID != "account_1" || payload.APIKeyID != "key_1" {
		t.Fatalf("unexpected auth result metadata: %#v", payload)
	}
	if payload.Success == nil || *payload.Success || payload.Retryable == nil || !*payload.Retryable {
		t.Fatalf("failure details should be preserved: %#v", payload)
	}
	if payload.HTTPStatus != http.StatusGatewayTimeout || payload.ErrorCode != "upstream_timeout" {
		t.Fatalf("unexpected failure details: %#v", payload)
	}
	if payload.AuthAvailable == nil || *payload.AuthAvailable || payload.NextRetryAtMS <= time.Now().UnixMilli() || payload.AuthStateReason != "unauthorized" {
		t.Fatalf("scheduler state should be preserved: %#v", payload)
	}
}

func TestRelayServerExecutesNonStreamingRequestThroughRuntime(t *testing.T) {
	gin.SetMode(gin.TestMode)
	runtime := &fakeRuntime{
		response: cliproxyexecutor.Response{
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Payload: []byte(`{"ok":true}`),
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.5","input":"hello","stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if strings.TrimSpace(w.Body.String()) != `{"ok":true}` {
		t.Fatalf("unexpected body: %s", w.Body.String())
	}
	if runtime.executeCalls != 1 || runtime.streamCalls != 0 {
		t.Fatalf("unexpected runtime calls: execute=%d stream=%d", runtime.executeCalls, runtime.streamCalls)
	}
	if runtime.lastReq.Model != "gpt-5.5" || runtime.lastOpts.SourceFormat != sdktranslator.FormatOpenAIResponse {
		t.Fatalf("unexpected executor request: %#v %#v", runtime.lastReq, runtime.lastOpts)
	}
	if runtime.lastOpts.Headers.Get("Authorization") != "Bearer client-key" {
		t.Fatalf("request headers should be forwarded to CPA executor")
	}
	if w.Header().Get("Access-Control-Allow-Origin") != "*" {
		t.Fatalf("CORS header should match CPA server behavior")
	}
}

func TestRelayServerRejectsGPTImageModelsOnChatCompletionsBeforeRuntime(t *testing.T) {
	gin.SetMode(gin.TestMode)
	runtime := &fakeRuntime{}
	apiKey := &apiKeySpec{
		ID:          "key_1",
		Label:       "Test key",
		Key:         "client-key",
		ModelPrefix: "team",
		Enabled:     true,
	}
	m := &manifest{
		APIKeys:  []apiKeySpec{*apiKey},
		ModelIDs: []string{"gpt-5.5", "gpt-image-2"},
		ModelAliases: []modelAliasSpec{{
			SourceModel: "gpt-image-2",
			Alias:       "image-latest",
		}},
		apiKeyByValue: map[string]*apiKeySpec{"client-key": apiKey},
		aliasToSource: map[string]string{"image-latest": "gpt-image-2"},
	}
	router := (&relayServer{
		runtime:  runtime,
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	for _, model := range []string{"team/gpt-image-2", "team/image-latest"} {
		req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(fmt.Sprintf(`{"model":%q,"messages":[{"role":"user","content":"draw"}]}`, model)))
		req.Header.Set("Authorization", "Bearer client-key")
		req.Header.Set("Content-Type", "application/json")
		w := httptest.NewRecorder()

		router.ServeHTTP(w, req)

		if w.Code != http.StatusBadRequest {
			t.Fatalf("model %q status = %d, want %d; body=%s", model, w.Code, http.StatusBadRequest, w.Body.String())
		}
		var payload struct {
			Error struct {
				Message string `json:"message"`
				Type    string `json:"type"`
			} `json:"error"`
		}
		if err := json.Unmarshal(w.Body.Bytes(), &payload); err != nil {
			t.Fatalf("model %q response should be JSON: %v", model, err)
		}
		if payload.Error.Type != "invalid_request_error" || !strings.Contains(payload.Error.Message, "Chat Completions") {
			t.Fatalf("model %q unexpected error payload: %#v", model, payload.Error)
		}
	}

	if runtime.executeCalls != 0 || runtime.streamCalls != 0 {
		t.Fatalf("image-only models must be rejected before runtime scheduling: execute=%d stream=%d", runtime.executeCalls, runtime.streamCalls)
	}
}

func TestRelayServerProviderGatewayRoutesResponsesToChatCompletions(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var upstreamPath string
	var upstreamAuth string
	var upstreamBody string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		upstreamPath = r.URL.Path
		upstreamAuth = r.Header.Get("Authorization")
		body, _ := io.ReadAll(r.Body)
		upstreamBody = string(body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"deepseek-chat","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}`))
	}))
	defer upstream.Close()

	runtime := &fakeRuntime{}
	m := &manifest{
		APIKeys: []apiKeySpec{{
			ID:      "provider_gateway_account_1",
			Label:   "Provider Gateway",
			Key:     "client-key",
			Enabled: true,
			ProviderGateway: &providerGatewaySpec{
				BaseURL:        upstream.URL,
				APIKey:         "deepseek-key",
				UpstreamModel:  "deepseek-v4-flash",
				UpstreamModels: []string{"deepseek-v4-flash", "deepseek-v4-pro"},
				WireAPI:        "chat_completions",
			},
		}},
		ModelIDs: []string{"deepseek-chat"},
		ModelAliases: []modelAliasSpec{
			{SourceModel: "deepseek-v4-flash", Alias: "gpt-5.5"},
			{SourceModel: "deepseek-v4-pro", Alias: "gpt-5.4"},
		},
		aliasToSource: map[string]string{
			"gpt-5.5": "deepseek-v4-flash",
			"gpt-5.4": "deepseek-v4-pro",
		},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {
				ID:      "provider_gateway_account_1",
				Label:   "Provider Gateway",
				Key:     "client-key",
				Enabled: true,
				ProviderGateway: &providerGatewaySpec{
					BaseURL:        upstream.URL,
					APIKey:         "deepseek-key",
					UpstreamModel:  "deepseek-v4-flash",
					UpstreamModels: []string{"deepseek-v4-flash", "deepseek-v4-pro"},
					WireAPI:        "chat_completions",
				},
			},
		},
	}
	router := (&relayServer{
		runtime:  runtime,
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.4","input":"hello","stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.executeCalls != 0 || runtime.streamCalls != 0 {
		t.Fatalf("provider gateway should bypass runtime auth pool: execute=%d stream=%d", runtime.executeCalls, runtime.streamCalls)
	}
	if upstreamPath != "/v1/chat/completions" {
		t.Fatalf("unexpected upstream path: %s", upstreamPath)
	}
	if upstreamAuth != "Bearer deepseek-key" {
		t.Fatalf("unexpected upstream auth: %s", upstreamAuth)
	}
	if !strings.Contains(upstreamBody, `"messages"`) || !strings.Contains(upstreamBody, `"stream":false`) {
		t.Fatalf("request should be converted to chat completions: %s", upstreamBody)
	}
	if !strings.Contains(upstreamBody, `"model":"deepseek-v4-pro"`) || strings.Contains(upstreamBody, `"model":"gpt-5.4"`) {
		t.Fatalf("request should use provider upstream model: %s", upstreamBody)
	}
	if !strings.Contains(w.Body.String(), `"object":"response"`) || !strings.Contains(w.Body.String(), `"output_text"`) {
		t.Fatalf("response should be converted back to responses shape: %s", w.Body.String())
	}

	modelReq := httptest.NewRequest(http.MethodGet, "/v1/models?codex_client=1", nil)
	modelReq.Header.Set("Authorization", "Bearer client-key")
	modelW := httptest.NewRecorder()
	router.ServeHTTP(modelW, modelReq)
	if modelW.Code != http.StatusOK {
		t.Fatalf("unexpected models status: %d body=%s", modelW.Code, modelW.Body.String())
	}
	if !strings.Contains(modelW.Body.String(), "gpt-5.5") || !strings.Contains(modelW.Body.String(), "gpt-5.4") || strings.Contains(modelW.Body.String(), "deepseek-v4-pro") {
		t.Fatalf("provider gateway should expose client model slots only: %s", modelW.Body.String())
	}
}

func TestRelayServerProviderGatewayPreservesVersionedBaseURL(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var upstreamPath string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		upstreamPath = r.URL.Path
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"glm-5.1","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:        upstream.URL + "/api/coding/paas/v4",
		APIKey:         "zhipu-key",
		UpstreamModel:  "glm-5.1",
		UpstreamModels: []string{"glm-5.1"},
		WireAPI:        "chat_completions",
	}
	m := &manifest{
		APIKeys:  []apiKeySpec{{ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway}},
		ModelIDs: []string{"glm-5.1"},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway},
		},
	}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"glm-5.1","input":"hello","stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if upstreamPath != "/api/coding/paas/v4/chat/completions" {
		t.Fatalf("unexpected upstream path: %s", upstreamPath)
	}
}

func TestProviderGatewayURLPreservesVersionedBasePaths(t *testing.T) {
	tests := []struct {
		name string
		base string
		path string
		want string
	}{
		{
			name: "bare host appends openai v1 path",
			base: "https://api.example.com",
			path: "/v1/chat/completions",
			want: "https://api.example.com/v1/chat/completions",
		},
		{
			name: "existing v1 base keeps single v1",
			base: "https://api.example.com/v1/",
			path: "/v1/chat/completions",
			want: "https://api.example.com/v1/chat/completions",
		},
		{
			name: "complete endpoint is left unchanged",
			base: "https://api.example.com/v1/chat/completions",
			path: "/v1/chat/completions",
			want: "https://api.example.com/v1/chat/completions",
		},
		{
			name: "zhipu coding paas v4 base keeps v4 root",
			base: "https://open.bigmodel.cn/api/coding/paas/v4",
			path: "/v1/chat/completions",
			want: "https://open.bigmodel.cn/api/coding/paas/v4/chat/completions",
		},
		{
			name: "zai coding paas v4 base keeps v4 root",
			base: "https://api.z.ai/api/coding/paas/v4",
			path: "/v1/chat/completions",
			want: "https://api.z.ai/api/coding/paas/v4/chat/completions",
		},
		{
			name: "volcengine coding v3 base keeps v3 root",
			base: "https://ark.cn-beijing.volces.com/api/coding/v3",
			path: "/v1/chat/completions",
			want: "https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions",
		},
		{
			name: "doubao api v3 base keeps v3 root",
			base: "https://ark.cn-beijing.volces.com/api/v3",
			path: "/v1/chat/completions",
			want: "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
		},
		{
			name: "qianfan v2 coding base keeps v2 root",
			base: "https://qianfan.baidubce.com/v2/coding",
			path: "/v1/chat/completions",
			want: "https://qianfan.baidubce.com/v2/coding/chat/completions",
		},
		{
			name: "versioned responses path drops openai v1 prefix",
			base: "https://open.bigmodel.cn/api/coding/paas/v4",
			path: "/v1/responses",
			want: "https://open.bigmodel.cn/api/coding/paas/v4/responses",
		},
		{
			name: "base query is stripped",
			base: "https://api.example.com/v1?ignored=1",
			path: "/v1/chat/completions",
			want: "https://api.example.com/v1/chat/completions",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := providerGatewayURL(tt.base, tt.path)
			if err != nil {
				t.Fatalf("providerGatewayURL returned error: %v", err)
			}
			if got != tt.want {
				t.Fatalf("providerGatewayURL() = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestRelayServerProviderGatewayChatStreamTerminatesResponsesSSEFrames(t *testing.T) {
	gin.SetMode(gin.TestMode)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = io.WriteString(w, "data: {\"id\":\"chatcmpl_1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"deepseek-v4-flash\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"ok\"},\"finish_reason\":null}]}\n\n")
		_, _ = io.WriteString(w, "data: [DONE]\n\n")
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:        upstream.URL,
		APIKey:         "deepseek-key",
		UpstreamModel:  "deepseek-v4-flash",
		UpstreamModels: []string{"deepseek-v4-flash"},
		WireAPI:        "chat_completions",
		SupportsVision: true,
	}
	m := &manifest{
		APIKeys:  []apiKeySpec{{ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway}},
		ModelIDs: []string{"deepseek-v4-flash"},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway},
		},
	}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"deepseek-v4-flash","input":"hello","stream":true}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	body := w.Body.String()
	if !strings.Contains(body, "event: response.completed") {
		t.Fatalf("stream should include response.completed: %s", body)
	}
	if !strings.Contains(body, "event: response.completed\n") || !strings.Contains(body, "\n\n") {
		t.Fatalf("stream should emit complete SSE frames separated by a blank line: %q", body)
	}
}

func TestRelayServerProviderGatewayFallsBackToDefaultUpstreamModel(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var upstreamBody string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		upstreamBody = string(body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"deepseek-v4-flash","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:        upstream.URL,
		APIKey:         "deepseek-key",
		UpstreamModel:  "deepseek-v4-flash",
		UpstreamModels: []string{"deepseek-v4-flash", "deepseek-v4-pro"},
		WireAPI:        "chat_completions",
	}
	m := &manifest{
		APIKeys:  []apiKeySpec{{ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway}},
		ModelIDs: []string{"deepseek-v4-flash", "deepseek-v4-pro"},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway},
		},
	}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.4","input":"hello","stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if !strings.Contains(upstreamBody, `"model":"deepseek-v4-flash"`) || strings.Contains(upstreamBody, `"model":"gpt-5.4"`) {
		t.Fatalf("request should fall back to provider default upstream model: %s", upstreamBody)
	}
}

func TestRelayServerProviderGatewayPassesThroughModelWhenCatalogEmpty(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var upstreamBody string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		upstreamBody = string(body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"gpt-5","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL: upstream.URL,
		APIKey:  "provider-key",
		WireAPI: "chat_completions",
	}
	m := &manifest{
		APIKeys:  []apiKeySpec{{ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway}},
		ModelIDs: []string{"gpt-5"},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway},
		},
	}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5","input":"hello","stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if !strings.Contains(upstreamBody, `"model":"gpt-5"`) || strings.Contains(upstreamBody, "gpt-5.5") {
		t.Fatalf("request should pass through the client model when provider catalog is empty: %s", upstreamBody)
	}
}

func TestRelayServerProviderGatewayUsesSelectedUpstreamModel(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var upstreamBody string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		upstreamBody = string(body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"deepseek-v4-pro","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:        upstream.URL,
		APIKey:         "deepseek-key",
		UpstreamModel:  "deepseek-v4-flash",
		UpstreamModels: []string{"deepseek-v4-flash", "deepseek-v4-pro"},
		WireAPI:        "chat_completions",
	}
	m := &manifest{
		APIKeys:  []apiKeySpec{{ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway}},
		ModelIDs: []string{"deepseek-v4-flash", "deepseek-v4-pro"},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway},
		},
	}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"deepseek-v4-pro","input":"hello","stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if !strings.Contains(upstreamBody, `"model":"deepseek-v4-pro"`) || strings.Contains(upstreamBody, `"model":"deepseek-v4-flash"`) {
		t.Fatalf("request should use selected upstream model: %s", upstreamBody)
	}
}

func TestRelayServerProviderGatewayOmitsVisionInputWhenUnsupported(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var upstreamBody string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		upstreamBody = string(body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"deepseek-v4-flash","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:        upstream.URL,
		APIKey:         "deepseek-key",
		UpstreamModel:  "deepseek-v4-flash",
		UpstreamModels: []string{"deepseek-v4-flash"},
		WireAPI:        "chat_completions",
	}
	m := &manifest{
		APIKeys:  []apiKeySpec{{ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway}},
		ModelIDs: []string{"deepseek-v4-flash"},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway},
		},
	}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"deepseek-v4-flash","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"describe"},{"type":"input_image","image_url":"data:image/png;base64,abc"}]}],"stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if upstreamBody == "" {
		t.Fatal("text-only fallback should call upstream")
	}
	if strings.Contains(upstreamBody, "image_url") || strings.Contains(upstreamBody, "data:image") {
		t.Fatalf("text-only fallback should omit image data: %s", upstreamBody)
	}
	if !strings.Contains(upstreamBody, providerGatewayOmittedImageText) {
		t.Fatalf("text-only fallback should explain the omitted image: %s", upstreamBody)
	}
	if !strings.Contains(upstreamBody, `"model":"deepseek-v4-flash"`) {
		t.Fatalf("text-only fallback should keep the selected model: %s", upstreamBody)
	}
}

func TestRelayServerProviderGatewayRoutesVisionInputToConfiguredModel(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var upstreamPath string
	var upstreamBody string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		upstreamPath = r.URL.Path
		body, _ := io.ReadAll(r.Body)
		upstreamBody = string(body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"mimo-v2.5","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:            upstream.URL,
		APIKey:             "mimo-key",
		UpstreamModel:      "mimo-v2.5-pro",
		UpstreamModels:     []string{"mimo-v2.5-pro", "mimo-v2.5"},
		WireAPI:            "chat_completions",
		VisionRoutingModel: "mimo-v2.5",
		ModelCapabilities: map[string]providerGatewayModelCapability{
			"mimo-v2.5": {SupportsVision: true},
		},
	}
	m := &manifest{
		APIKeys:  []apiKeySpec{{ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway}},
		ModelIDs: []string{"mimo-v2.5-pro", "mimo-v2.5"},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway},
		},
	}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"mimo-v2.5-pro","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"describe"},{"type":"input_image","image_url":"data:image/png;base64,abc"}]}],"stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if upstreamPath != "/v1/chat/completions" {
		t.Fatalf("unexpected upstream path: %s", upstreamPath)
	}
	if !strings.Contains(upstreamBody, `"model":"mimo-v2.5"`) || strings.Contains(upstreamBody, `"model":"mimo-v2.5-pro"`) {
		t.Fatalf("vision request should be routed to configured model: %s", upstreamBody)
	}
	if !strings.Contains(upstreamBody, "image_url") {
		t.Fatalf("vision request should keep image input: %s", upstreamBody)
	}
}

func TestRelayServerProviderGatewayRoutesVisionInputToOnlyVisionModel(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var upstreamBody string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		upstreamBody = string(body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"mimo-v2.5","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:        upstream.URL,
		APIKey:         "mimo-key",
		UpstreamModel:  "mimo-v2.5-pro",
		UpstreamModels: []string{"mimo-v2.5-pro", "mimo-v2.5"},
		WireAPI:        "chat_completions",
		ModelCapabilities: map[string]providerGatewayModelCapability{
			"mimo-v2.5": {SupportsVision: true},
		},
	}
	m := &manifest{
		APIKeys:  []apiKeySpec{{ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway}},
		ModelIDs: []string{"mimo-v2.5-pro", "mimo-v2.5"},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway},
		},
	}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"mimo-v2.5-pro","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"describe"},{"type":"input_image","image_url":"data:image/png;base64,abc"}]}],"stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if !strings.Contains(upstreamBody, `"model":"mimo-v2.5"`) || strings.Contains(upstreamBody, `"model":"mimo-v2.5-pro"`) {
		t.Fatalf("single vision model should be used automatically: %s", upstreamBody)
	}
}

func TestRelayServerProviderGatewayAllowsVisionInputForModelCapability(t *testing.T) {
	gin.SetMode(gin.TestMode)
	upstreamCalled := false
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		upstreamCalled = true
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"qwen-vl-plus","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:        upstream.URL,
		APIKey:         "qwen-key",
		UpstreamModel:  "qwen-plus",
		UpstreamModels: []string{"qwen-plus", "qwen-vl-plus"},
		WireAPI:        "chat_completions",
		ModelCapabilities: map[string]providerGatewayModelCapability{
			"qwen-vl-plus": {SupportsVision: true},
		},
	}
	m := &manifest{
		APIKeys:  []apiKeySpec{{ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway}},
		ModelIDs: []string{"qwen-plus", "qwen-vl-plus"},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway},
		},
	}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"qwen-vl-plus","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"describe"},{"type":"input_image","image_url":"data:image/png;base64,abc"}]}],"stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if !upstreamCalled {
		t.Fatal("vision-capable model should call upstream")
	}
}

func TestRelayServerProviderGatewayAllowsVisionInputForProviderDefault(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var upstreamBody string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		upstreamBody = string(body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"qwen-vl-plus","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:        upstream.URL,
		APIKey:         "qwen-key",
		UpstreamModel:  "qwen-vl-plus",
		UpstreamModels: []string{"qwen-vl-plus"},
		WireAPI:        "chat_completions",
		SupportsVision: true,
	}
	m := &manifest{
		APIKeys:  []apiKeySpec{{ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway}},
		ModelIDs: []string{"qwen-vl-plus"},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway},
		},
	}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"qwen-vl-plus","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"describe"},{"type":"input_image","image_url":"data:image/png;base64,abc"}]}],"stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if strings.Contains(upstreamBody, "Image omitted") || !strings.Contains(upstreamBody, "image_url") {
		t.Fatalf("provider default vision support should keep image input: %s", upstreamBody)
	}
}

func TestRelayServerAcceptsCodexAutoReviewModel(t *testing.T) {
	gin.SetMode(gin.TestMode)
	runtime := &fakeRuntime{
		response: cliproxyexecutor.Response{
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Payload: []byte(`{"ok":true}`),
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"codex-auto-review","input":"allow?","stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.executeCalls != 1 || runtime.lastReq.Model != codexAutoReviewModel {
		t.Fatalf("auto review request should be forwarded unchanged: calls=%d req=%#v", runtime.executeCalls, runtime.lastReq)
	}
}

func TestRelayServerModelsExposeCodexAutoReview(t *testing.T) {
	gin.SetMode(gin.TestMode)
	router := testRelayRouter(&fakeRuntime{})

	req := httptest.NewRequest(http.MethodGet, "/v1/models", nil)
	req.Header.Set("Authorization", "Bearer client-key")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), codexAutoReviewModel) {
		t.Fatalf("models response should expose auto review model: %s", w.Body.String())
	}
}

func TestRelayServerResetAuthStateClearsSelectedAccountCooldown(t *testing.T) {
	gin.SetMode(gin.TestMode)
	manager := coreauth.NewManager(nil, &coreauth.RoundRobinSelector{}, nil)
	if _, err := manager.Register(context.Background(), &coreauth.Auth{
		ID:       "auth-1.json",
		Provider: "codex",
		Metadata: map[string]any{"type": "codex"},
		ModelStates: map[string]*coreauth.ModelState{
			"gpt-5.5": {
				Status:         coreauth.StatusError,
				Unavailable:    true,
				NextRetryAfter: time.Now().Add(30 * time.Minute),
			},
		},
	}); err != nil {
		t.Fatalf("register auth: %v", err)
	}
	spec := &apiKeySpec{ID: "key_1", Key: "client-key", Enabled: true, AccountIDs: []string{"account-1"}}
	account := &accountSpec{ID: "account-1", AuthID: "auth-1.json"}
	m := &manifest{
		APIKeys:       []apiKeySpec{*spec},
		Accounts:      []accountSpec{*account},
		apiKeyByValue: map[string]*apiKeySpec{"client-key": spec},
		accountByID:   map[string]*accountSpec{"account-1": account},
	}
	router := (&relayServer{
		runtime:     &fakeRuntime{},
		cfg:         &config.Config{},
		manifest:    m,
		authManager: manager,
		policy:      &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodPost, "/v1/cockpit/auth/reset", strings.NewReader(`{"accountIds":["account-1"]}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	updated, ok := manager.GetByID("auth-1.json")
	if !ok || updated == nil || len(updated.ModelStates) != 0 || updated.Unavailable {
		t.Fatalf("auth state was not reset: %#v", updated)
	}
}

func TestRelayServerFramesStreamingChatCompletionThroughRuntime(t *testing.T) {
	gin.SetMode(gin.TestMode)
	stream := make(chan cliproxyexecutor.StreamChunk, 2)
	stream <- cliproxyexecutor.StreamChunk{Payload: []byte(`{"choices":[]}`)}
	stream <- cliproxyexecutor.StreamChunk{Payload: []byte(`[DONE]`)}
	close(stream)
	runtime := &fakeRuntime{
		streamResult: &cliproxyexecutor.StreamResult{
			Headers: http.Header{
				"Content-Type":       []string{"application/json"},
				"Connection":         []string{"X-Remove-Me"},
				"X-Remove-Me":        []string{"secret"},
				"X-Litellm-Trace":    []string{"gateway"},
				"Content-Encoding":   []string{"gzip"},
				"X-Upstream":         []string{"ok"},
				"Access-Control-Foo": []string{"bar"},
			},
			Chunks: stream,
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(`{"model":"gpt-5.5","messages":[],"stream":true}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.executeCalls != 0 || runtime.streamCalls != 1 {
		t.Fatalf("unexpected runtime calls: execute=%d stream=%d", runtime.executeCalls, runtime.streamCalls)
	}
	if runtime.lastOpts.SourceFormat != sdktranslator.FormatOpenAI || !runtime.lastOpts.Stream {
		t.Fatalf("unexpected stream options: %#v", runtime.lastOpts)
	}
	if got := w.Header().Get("Content-Type"); !strings.HasPrefix(got, "text/event-stream") {
		t.Fatalf("unexpected content type: %q", got)
	}
	if values := w.Header().Values("Content-Type"); len(values) != 1 {
		t.Fatalf("Content-Type should not be duplicated: %#v", values)
	}
	if w.Header().Get("X-Upstream") != "ok" {
		t.Fatalf("upstream headers should be preserved")
	}
	if w.Header().Get("X-Remove-Me") != "" ||
		w.Header().Get("X-Litellm-Trace") != "" ||
		w.Header().Get("Content-Encoding") != "" {
		t.Fatalf("filtered upstream headers leaked: %#v", w.Header())
	}
	if got := w.Body.String(); got != "data: {\"choices\":[]}\n\ndata: [DONE]\n\n" {
		t.Fatalf("unexpected framed stream:\n%s", got)
	}
}

func TestRelayServerTimesOutWhenStreamDoesNotOpen(t *testing.T) {
	gin.SetMode(gin.TestMode)
	oldTimeout := streamOpenTimeout
	oldAttempts := streamOpenMaxAttempts
	streamOpenTimeout = 20 * time.Millisecond
	streamOpenMaxAttempts = 2
	defer func() {
		streamOpenTimeout = oldTimeout
		streamOpenMaxAttempts = oldAttempts
	}()
	router := testRelayRouter(&fakeRuntime{streamWaitForContext: true})

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.5","input":"hello","stream":true}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusGatewayTimeout {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "stream_open") {
		t.Fatalf("timeout response should name stream_open phase: %s", w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "upstream_first_byte_timeout") {
		t.Fatalf("timeout response should expose first-byte timeout code: %s", w.Body.String())
	}
}

func TestRelayServerUsesLongOpenTimeoutForImageGenerationTool(t *testing.T) {
	gin.SetMode(gin.TestMode)
	oldOpenTimeout := streamOpenTimeout
	oldImageOpenTimeout := imageStreamOpenTimeout
	oldAttempts := streamOpenMaxAttempts
	streamOpenTimeout = 20 * time.Millisecond
	imageStreamOpenTimeout = 120 * time.Millisecond
	streamOpenMaxAttempts = 1
	defer func() {
		streamOpenTimeout = oldOpenTimeout
		imageStreamOpenTimeout = oldImageOpenTimeout
		streamOpenMaxAttempts = oldAttempts
	}()
	stream := make(chan cliproxyexecutor.StreamChunk, 1)
	stream <- cliproxyexecutor.StreamChunk{Payload: []byte(`event: response.completed
data: {"type":"response.completed"}

`)}
	close(stream)
	runtime := &fakeRuntime{
		streamOpenDelay: 60 * time.Millisecond,
		streamResult: &cliproxyexecutor.StreamResult{
			Headers: http.Header{"Content-Type": []string{"text/event-stream"}},
			Chunks:  stream,
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.5","input":"draw","stream":true,"tools":[{"type":"image_generation","model":"gpt-image-2"}]}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("image stream should use longer open timeout, got status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.streamCalls != 1 {
		t.Fatalf("expected one stream runtime call, got %d", runtime.streamCalls)
	}
	if !strings.Contains(w.Body.String(), "response.completed") {
		t.Fatalf("image stream response was not forwarded: %s", w.Body.String())
	}
}

func TestRelayServerHandlesImagesGenerationsEndpoint(t *testing.T) {
	gin.SetMode(gin.TestMode)
	stream := make(chan cliproxyexecutor.StreamChunk, 1)
	stream <- cliproxyexecutor.StreamChunk{Payload: []byte(`event: response.completed
data: {"type":"response.completed","response":{"created_at":1710000000,"output":[{"type":"image_generation_call","result":"ZmFrZS1wbmc=","output_format":"png","size":"1024x1024"}]}}

`)}
	close(stream)
	runtime := &fakeRuntime{
		streamResult: &cliproxyexecutor.StreamResult{
			Headers: http.Header{"Content-Type": []string{"text/event-stream"}},
			Chunks:  stream,
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/images/generations", strings.NewReader(`{"model":"gpt-image-2","prompt":"draw","response_format":"b64_json"}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.streamCalls != 1 || runtime.executeCalls != 0 {
		t.Fatalf("unexpected runtime calls: execute=%d stream=%d", runtime.executeCalls, runtime.streamCalls)
	}
	if runtime.lastReq.Model != defaultImagesMainModel {
		t.Fatalf("image endpoint should execute via main model, got %q", runtime.lastReq.Model)
	}
	var body map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &body); err != nil {
		t.Fatalf("response should be json: %v body=%s", err, w.Body.String())
	}
	data, _ := body["data"].([]any)
	if len(data) != 1 {
		t.Fatalf("expected one image result: %#v", body)
	}
	first, _ := data[0].(map[string]any)
	if first["b64_json"] != "ZmFrZS1wbmc=" {
		t.Fatalf("unexpected image payload: %#v", body)
	}
}

func TestRelayServerRetriesWhenStreamDoesNotOpen(t *testing.T) {
	gin.SetMode(gin.TestMode)
	oldTimeout := streamOpenTimeout
	oldAttempts := streamOpenMaxAttempts
	streamOpenTimeout = 20 * time.Millisecond
	streamOpenMaxAttempts = 2
	defer func() {
		streamOpenTimeout = oldTimeout
		streamOpenMaxAttempts = oldAttempts
	}()
	stream := make(chan cliproxyexecutor.StreamChunk, 1)
	stream <- cliproxyexecutor.StreamChunk{Payload: []byte(`[DONE]`)}
	close(stream)
	runtime := &fakeRuntime{
		streamWaitAttempts: 1,
		streamResult: &cliproxyexecutor.StreamResult{
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Chunks:  stream,
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.5","input":"hello","stream":true}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.streamCalls != 2 {
		t.Fatalf("expected retry to call stream runtime twice, got %d", runtime.streamCalls)
	}
	if !strings.Contains(w.Body.String(), "[DONE]") {
		t.Fatalf("retry should stream successful second attempt: %s", w.Body.String())
	}
}

func TestRelayServerKeepsStreamContextOpenAfterOpen(t *testing.T) {
	gin.SetMode(gin.TestMode)
	oldOpenTimeout := streamOpenTimeout
	oldIdleTimeout := streamIdleTimeout
	streamOpenTimeout = 100 * time.Millisecond
	streamIdleTimeout = time.Second
	defer func() {
		streamOpenTimeout = oldOpenTimeout
		streamIdleTimeout = oldIdleTimeout
	}()
	runtime := &fakeRuntime{
		streamResultFromContext: true,
		streamResultDelay:       20 * time.Millisecond,
		streamResultPayload:     []byte(`[DONE]`),
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.5","input":"hello","stream":true}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.streamCalls != 1 {
		t.Fatalf("expected one stream runtime call, got %d", runtime.streamCalls)
	}
	if !strings.Contains(w.Body.String(), "[DONE]") {
		t.Fatalf("stream context should stay alive after opening: %s", w.Body.String())
	}
}

func TestRelayServerTimesOutIdleOpenedStream(t *testing.T) {
	gin.SetMode(gin.TestMode)
	oldTimeout := streamIdleTimeout
	streamIdleTimeout = 20 * time.Millisecond
	defer func() {
		streamIdleTimeout = oldTimeout
	}()
	stream := make(chan cliproxyexecutor.StreamChunk)
	runtime := &fakeRuntime{
		streamResult: &cliproxyexecutor.StreamResult{
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Chunks:  stream,
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.5","input":"hello","stream":true}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("stream should be opened before idle timeout, got status: %d body=%s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "stream_idle") {
		t.Fatalf("idle timeout should be sent as terminal SSE error: %s", w.Body.String())
	}
}

func TestRelayServerAnthropicMessagesUsesClaudeFormat(t *testing.T) {
	gin.SetMode(gin.TestMode)
	runtime := &fakeRuntime{
		response: cliproxyexecutor.Response{
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Payload: []byte(`{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"text","text":"ok"}]}`),
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/messages", strings.NewReader(`{"model":"gpt-5.5","messages":[{"role":"user","content":"hello"}],"stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.executeCalls != 1 || runtime.lastOpts.SourceFormat != sdktranslator.FormatClaude || runtime.lastReq.Format != sdktranslator.FormatClaude {
		t.Fatalf("expected Claude executor request, got calls=%d req=%#v opts=%#v", runtime.executeCalls, runtime.lastReq, runtime.lastOpts)
	}
}

func TestRelayServerAnthropicCountTokensUsesClaudeShape(t *testing.T) {
	gin.SetMode(gin.TestMode)
	router := testRelayRouter(&fakeRuntime{})

	req := httptest.NewRequest(http.MethodPost, "/v1/messages/count_tokens", strings.NewReader(`{"model":"gpt-5.5","messages":[{"role":"user","content":"hello world"}]}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), `"input_tokens"`) {
		t.Fatalf("Anthropic token count response should use input_tokens: %s", w.Body.String())
	}
}

func TestRelayServerGeminiGenerateInjectsPathModel(t *testing.T) {
	gin.SetMode(gin.TestMode)
	runtime := &fakeRuntime{
		response: cliproxyexecutor.Response{
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Payload: []byte(`{"candidates":[{"content":{"role":"model","parts":[{"text":"ok"}]}}]}`),
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1beta/models/gpt-5.5:generateContent", strings.NewReader(`{"contents":[{"role":"user","parts":[{"text":"hello"}]}]}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.executeCalls != 1 || runtime.lastOpts.SourceFormat != sdktranslator.FormatGemini || runtime.lastReq.Model != "gpt-5.5" {
		t.Fatalf("expected Gemini executor request, got calls=%d req=%#v opts=%#v", runtime.executeCalls, runtime.lastReq, runtime.lastOpts)
	}
	if !strings.Contains(string(runtime.lastReq.Payload), `"model":"gpt-5.5"`) {
		t.Fatalf("Gemini path model should be injected into executor payload: %s", runtime.lastReq.Payload)
	}
}

func TestRelayServerGeminiModelsResponseShape(t *testing.T) {
	gin.SetMode(gin.TestMode)
	router := testRelayRouter(&fakeRuntime{})

	req := httptest.NewRequest(http.MethodGet, "/v1beta/models", nil)
	req.Header.Set("Authorization", "Bearer client-key")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), `"name":"models/gpt-5.5"`) ||
		!strings.Contains(w.Body.String(), `"streamGenerateContent"`) ||
		!strings.Contains(w.Body.String(), `"countTokens"`) {
		t.Fatalf("Gemini models response has unexpected shape: %s", w.Body.String())
	}
}

func TestRelayServerOllamaChatConvertsNonStreamingResponse(t *testing.T) {
	gin.SetMode(gin.TestMode)
	runtime := &fakeRuntime{
		response: cliproxyexecutor.Response{
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Payload: []byte(`{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"gpt-5.5","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}}`),
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/api/chat", strings.NewReader(`{"model":"gpt-5.5","messages":[{"role":"user","content":"hello"}],"stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.executeCalls != 1 || runtime.lastOpts.SourceFormat != sdktranslator.FormatOpenAI || runtime.lastReq.Model != "gpt-5.5" {
		t.Fatalf("expected OpenAI chat executor request, got calls=%d req=%#v opts=%#v", runtime.executeCalls, runtime.lastReq, runtime.lastOpts)
	}
	if !strings.Contains(w.Body.String(), `"done":true`) || !strings.Contains(w.Body.String(), `"content":"ok"`) || !strings.Contains(w.Body.String(), `"eval_count":3`) {
		t.Fatalf("Ollama response has unexpected shape: %s", w.Body.String())
	}
}

func TestRelayServerOllamaChatConvertsStreamingChunks(t *testing.T) {
	gin.SetMode(gin.TestMode)
	chunks := make(chan cliproxyexecutor.StreamChunk, 2)
	chunks <- cliproxyexecutor.StreamChunk{Payload: []byte(`{"id":"chatcmpl_1","object":"chat.completion.chunk","created":1,"model":"gpt-5.5","choices":[{"index":0,"delta":{"role":"assistant","content":"ok"},"finish_reason":null}]}`)}
	chunks <- cliproxyexecutor.StreamChunk{Payload: []byte(`{"id":"chatcmpl_1","object":"chat.completion.chunk","created":1,"model":"gpt-5.5","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}}`)}
	close(chunks)
	runtime := &fakeRuntime{
		streamResult: &cliproxyexecutor.StreamResult{
			Headers: http.Header{"Content-Type": []string{"text/event-stream"}},
			Chunks:  chunks,
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/api/chat", strings.NewReader(`{"model":"gpt-5.5","messages":[{"role":"user","content":"hello"}]}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.streamCalls != 1 || runtime.lastOpts.SourceFormat != sdktranslator.FormatOpenAI {
		t.Fatalf("expected OpenAI chat stream executor request, got calls=%d opts=%#v", runtime.streamCalls, runtime.lastOpts)
	}
	lines := strings.Split(strings.TrimSpace(w.Body.String()), "\n")
	if len(lines) != 2 {
		t.Fatalf("expected content and final Ollama chunks, got %d lines: %s", len(lines), w.Body.String())
	}
	if !strings.Contains(lines[0], `"content":"ok"`) || !strings.Contains(lines[1], `"done":true`) || !strings.Contains(lines[1], `"eval_count":3`) {
		t.Fatalf("unexpected Ollama stream body: %s", w.Body.String())
	}
}

func TestRelayServerHandlesCORSPreflight(t *testing.T) {
	gin.SetMode(gin.TestMode)
	router := testRelayRouter(&fakeRuntime{})

	req := httptest.NewRequest(http.MethodOptions, "/v1/responses", nil)
	req.Header.Set("Access-Control-Request-Headers", "authorization,content-type")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusNoContent {
		t.Fatalf("unexpected status: %d", w.Code)
	}
	if w.Header().Get("Access-Control-Allow-Origin") != "*" ||
		w.Header().Get("Access-Control-Allow-Headers") != "*" {
		t.Fatalf("unexpected CORS headers: %#v", w.Header())
	}
}

func testRelayRouter(runtime executorRuntime) *gin.Engine {
	m := &manifest{
		APIKeys:  []apiKeySpec{{ID: "key_1", Label: "Test key", Key: "client-key", Enabled: true}},
		ModelIDs: []string{"gpt-5.5", "gpt-image-2"},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "key_1", Label: "Test key", Key: "client-key", Enabled: true},
		},
	}
	policy := &requestPolicy{manifest: m}
	return (&relayServer{
		runtime:  runtime,
		cfg:      &config.Config{},
		manifest: m,
		policy:   policy,
	}).router()
}

type fakeRuntime struct {
	response                cliproxyexecutor.Response
	streamResult            *cliproxyexecutor.StreamResult
	err                     error
	streamWaitForContext    bool
	streamWaitAttempts      int
	streamResultFromContext bool
	streamOpenDelay         time.Duration
	streamResultDelay       time.Duration
	streamResultPayload     []byte

	executeCalls int
	streamCalls  int
	lastReq      cliproxyexecutor.Request
	lastOpts     cliproxyexecutor.Options

	alphaSearchStatus  int
	alphaSearchHeaders http.Header
	alphaSearchPayload []byte
	alphaSearchErr     error
	alphaSearchCalls   int
	lastAlphaModel     string
	lastAlphaBody      []byte
	lastAlphaHeaders   http.Header
}

func (r *fakeRuntime) Execute(_ context.Context, _ []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	r.executeCalls++
	r.lastReq = req
	r.lastOpts = opts
	return r.response, r.err
}

func (r *fakeRuntime) CodexAlphaSearch(_ context.Context, model string, body []byte, headers http.Header) (int, http.Header, []byte, error) {
	r.alphaSearchCalls++
	r.lastAlphaModel = model
	r.lastAlphaBody = append([]byte(nil), body...)
	if headers != nil {
		r.lastAlphaHeaders = headers.Clone()
	}
	status := r.alphaSearchStatus
	if status == 0 {
		status = http.StatusOK
	}
	payload := r.alphaSearchPayload
	if payload == nil {
		payload = []byte(`{"ok":true}`)
	}
	return status, r.alphaSearchHeaders, payload, r.alphaSearchErr
}

func (r *fakeRuntime) ExecuteStream(ctx context.Context, _ []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (*cliproxyexecutor.StreamResult, error) {
	r.streamCalls++
	r.lastReq = req
	r.lastOpts = opts
	if r.streamWaitForContext || r.streamCalls <= r.streamWaitAttempts {
		<-ctx.Done()
		return nil, ctx.Err()
	}
	if r.streamOpenDelay > 0 {
		timer := time.NewTimer(r.streamOpenDelay)
		defer timer.Stop()
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-timer.C:
		}
	}
	if r.streamResultFromContext {
		stream := make(chan cliproxyexecutor.StreamChunk, 1)
		delay := r.streamResultDelay
		if delay <= 0 {
			delay = 10 * time.Millisecond
		}
		payload := r.streamResultPayload
		if len(payload) == 0 {
			payload = []byte(`[DONE]`)
		}
		go func() {
			defer close(stream)
			timer := time.NewTimer(delay)
			defer timer.Stop()
			select {
			case <-ctx.Done():
				return
			case <-timer.C:
				stream <- cliproxyexecutor.StreamChunk{Payload: payload}
			}
		}()
		return &cliproxyexecutor.StreamResult{
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Chunks:  stream,
		}, nil
	}
	return r.streamResult, r.err
}

func captureStdout(t *testing.T, fn func()) string {
	t.Helper()
	old := os.Stdout
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatalf("create stdout pipe: %v", err)
	}
	os.Stdout = writer
	defer func() {
		os.Stdout = old
		_ = reader.Close()
	}()

	fn()
	if err := writer.Close(); err != nil {
		t.Fatalf("close stdout pipe: %v", err)
	}
	data, err := io.ReadAll(reader)
	if err != nil {
		t.Fatalf("read stdout pipe: %v", err)
	}
	return string(data)
}

func TestRelayAcceptsResponsesPathAppendedToChatCompletionsBase(t *testing.T) {
	t.Parallel()
	// Route registration only: ensure compatibility paths are not NoRoute 404.
	m := &manifest{}
	policy := &requestPolicy{manifest: m}
	router := (&relayServer{
		manifest: m,
		policy:   policy,
	}).router()
	for _, path := range []string{
		"/v1/chat/completions/v1/responses",
		"/v1/chat/completions/v1/responses/compact",
	} {
		req := httptest.NewRequest(http.MethodPost, path, strings.NewReader(`{}`))
		req.Header.Set("Authorization", "Bearer unused")
		w := httptest.NewRecorder()
		router.ServeHTTP(w, req)
		if w.Code == http.StatusNotFound {
			t.Fatalf("path %s should not be NoRoute 404 (got %d body=%s)", path, w.Code, w.Body.String())
		}
	}
}

func TestSanitizeCodexAlphaSearchBodyRemovesLocalRoutingFields(t *testing.T) {
	t.Parallel()
	body := []byte(`{"query":"hello","prompt_cache_key":"drop-me","prompt_cache_retention":"24h","id":"sess-1"}`)
	out := sanitizeCodexAlphaSearchBody(body)
	if strings.Contains(string(out), "prompt_cache_key") || strings.Contains(string(out), "prompt_cache_retention") {
		t.Fatalf("local routing fields survived: %s", out)
	}
	if !strings.Contains(string(out), `"query":"hello"`) || !strings.Contains(string(out), `"id":"sess-1"`) {
		t.Fatalf("expected search fields preserved: %s", out)
	}
}

func TestResolveCodexAlphaSearchURL(t *testing.T) {
	t.Parallel()
	if got := resolveCodexAlphaSearchURL(nil); got != defaultCodexAlphaSearchURL {
		t.Fatalf("nil auth = %q, want default", got)
	}
	auth := &coreauth.Auth{Attributes: map[string]string{"base_url": "https://example.test/backend-api/codex/"}}
	if got := resolveCodexAlphaSearchURL(auth); got != "https://example.test/backend-api/codex/alpha/search" {
		t.Fatalf("codex base = %q", got)
	}
	auth.Attributes["base_url"] = "https://example.test/backend-api"
	if got := resolveCodexAlphaSearchURL(auth); got != "https://example.test/backend-api/codex/alpha/search" {
		t.Fatalf("backend-api base = %q", got)
	}
}

func TestRequestKindFromPathTreatsAlphaSearchAsText(t *testing.T) {
	t.Parallel()
	if got := requestKindFromPath("/v1/alpha/search"); got != "text" {
		t.Fatalf("requestKindFromPath(/v1/alpha/search) = %q, want text", got)
	}
	if got := requestKindFromPath("/backend-api/codex/alpha/search"); got != "text" {
		t.Fatalf("requestKindFromPath(direct) = %q, want text", got)
	}
}

func TestCodexAlphaSearchRouteForwardsToRuntime(t *testing.T) {
	t.Parallel()
	runtime := &fakeRuntime{
		alphaSearchStatus:  http.StatusOK,
		alphaSearchPayload: []byte(`{"results":[{"title":"ok"}]}`),
		alphaSearchHeaders: http.Header{"Content-Type": []string{"application/json"}},
	}
	m := &manifest{
		APIKeys: []apiKeySpec{{
			ID:      "key_1",
			Label:   "Test key",
			Key:     "client-key",
			Enabled: true,
		}},
		ModelIDs: []string{"gpt-5.6-sol"},
	}
	m.apiKeyByValue = map[string]*apiKeySpec{
		"client-key": &m.APIKeys[0],
	}
	router := (&relayServer{
		runtime:  runtime,
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	body := `{"query":"OpenAI Codex authentication documentation","model":"gpt-5.6-sol","id":"sess-42"}`
	req := httptest.NewRequest(http.MethodPost, codexAlphaSearchPath, strings.NewReader(body))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("X-Openai-Actor-Authorization", "actor-token")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", w.Code, w.Body.String())
	}
	if runtime.alphaSearchCalls != 1 {
		t.Fatalf("alphaSearchCalls = %d, want 1", runtime.alphaSearchCalls)
	}
	if runtime.lastAlphaModel != "gpt-5.6-sol" {
		t.Fatalf("model = %q, want gpt-5.6-sol", runtime.lastAlphaModel)
	}
	if !strings.Contains(string(runtime.lastAlphaBody), `"query":"OpenAI Codex authentication documentation"`) {
		t.Fatalf("body not forwarded: %s", runtime.lastAlphaBody)
	}
	if got := runtime.lastAlphaHeaders.Get("X-Session-ID"); got != "sess-42" {
		t.Fatalf("X-Session-ID = %q, want sess-42", got)
	}
	if got := runtime.lastAlphaHeaders.Get("X-Openai-Actor-Authorization"); got != "actor-token" {
		t.Fatalf("actor header = %q", got)
	}
	if !strings.Contains(w.Body.String(), `"results"`) {
		t.Fatalf("response body missing results: %s", w.Body.String())
	}
}

func TestCodexAlphaSearchDirectPathIsRegistered(t *testing.T) {
	t.Parallel()
	runtime := &fakeRuntime{alphaSearchPayload: []byte(`{"ok":true}`)}
	m := &manifest{
		APIKeys: []apiKeySpec{{ID: "key_1", Key: "client-key", Enabled: true, ResponsesWebsockets: true}},
	}
	m.apiKeyByValue = map[string]*apiKeySpec{"client-key": &m.APIKeys[0]}
	router := (&relayServer{
		runtime:  runtime,
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodPost, codexDirectAlphaSearchPath, strings.NewReader(`{"query":"ping"}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	if w.Code == http.StatusNotFound {
		t.Fatalf("direct path should not be NoRoute 404: %s", w.Body.String())
	}
	if runtime.alphaSearchCalls != 1 {
		t.Fatalf("alphaSearchCalls = %d, want 1", runtime.alphaSearchCalls)
	}
}

func TestCodexAlphaSearchRequiresAPIKey(t *testing.T) {
	t.Parallel()
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		manifest: &manifest{},
		policy:   &requestPolicy{manifest: &manifest{}},
	}).router()
	req := httptest.NewRequest(http.MethodPost, codexAlphaSearchPath, strings.NewReader(`{"query":"ping"}`))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want 401 body=%s", w.Code, w.Body.String())
	}
}

func TestResponsesWebsocketRouteRequiresAPIKey(t *testing.T) {
	t.Parallel()
	called := false
	m := &manifest{
		APIKeys: []apiKeySpec{{
			ID:                  "key_1",
			Key:                 "client-key",
			Enabled:             true,
			ResponsesWebsockets: true,
		}},
	}
	m.apiKeyByValue = map[string]*apiKeySpec{"client-key": &m.APIKeys[0]}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
		responsesWebsocket: func(c *gin.Context) {
			called = true
			c.Status(http.StatusSwitchingProtocols)
		},
	}).router()

	// Missing key → 401, handler not invoked.
	req := httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
	req.Header.Set("Connection", "Upgrade")
	req.Header.Set("Upgrade", "websocket")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("missing key status = %d, want 401 body=%s", w.Code, w.Body.String())
	}
	if called {
		t.Fatal("websocket handler should not run without API key")
	}

	// Valid key → handler runs.
	req = httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Connection", "Upgrade")
	req.Header.Set("Upgrade", "websocket")
	w = httptest.NewRecorder()
	router.ServeHTTP(w, req)
	if !called {
		t.Fatal("websocket handler should run with valid API key")
	}
	if w.Code != http.StatusSwitchingProtocols {
		t.Fatalf("valid key status = %d, want 101 body=%s", w.Code, w.Body.String())
	}
}

func TestResponsesWebsocketRouteUnavailableWithoutHandler(t *testing.T) {
	t.Parallel()
	m := &manifest{
		APIKeys: []apiKeySpec{{ID: "key_1", Key: "client-key", Enabled: true, ResponsesWebsockets: true}},
	}
	m.apiKeyByValue = map[string]*apiKeySpec{"client-key": &m.APIKeys[0]}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()

	req := httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
	req.Header.Set("Authorization", "Bearer client-key")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	if w.Code != http.StatusServiceUnavailable {
		t.Fatalf("status = %d, want 503 body=%s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "responses websocket unavailable") {
		t.Fatalf("body = %s", w.Body.String())
	}
}

func TestResponsesWebsocketRouteDisabledByDefault(t *testing.T) {
	t.Parallel()
	called := false
	m := &manifest{
		APIKeys: []apiKeySpec{{ID: "key_1", Key: "client-key", Enabled: true}},
	}
	m.apiKeyByValue = map[string]*apiKeySpec{"client-key": &m.APIKeys[0]}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
		responsesWebsocket: func(c *gin.Context) {
			called = true
			c.Status(http.StatusSwitchingProtocols)
		},
	}).router()

	req := httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
	req.Header.Set("Authorization", "Bearer client-key")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	if w.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want 400 body=%s", w.Code, w.Body.String())
	}
	if called {
		t.Fatal("websocket handler should not run when disabled")
	}
	if !strings.Contains(w.Body.String(), "responses websocket is disabled") {
		t.Fatalf("body = %s", w.Body.String())
	}
}

func TestResponsesWebsocketRejectsProviderGatewayBeforeCodexAuth(t *testing.T) {
	t.Parallel()
	called := false
	m := &manifest{
		APIKeys: []apiKeySpec{{
			ID:      "provider_gateway_deepseek",
			Key:     "client-key",
			Enabled: true,
			ProviderGateway: &providerGatewaySpec{
				BaseURL:       "https://api.deepseek.com/v1",
				APIKey:        "sk-deepseek",
				UpstreamModel: "deepseek-v4-pro",
				WireAPI:       "chat_completions",
			},
		}},
	}
	m.apiKeyByValue = map[string]*apiKeySpec{"client-key": &m.APIKeys[0]}
	router := (&relayServer{
		runtime:  &fakeRuntime{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
		responsesWebsocket: func(c *gin.Context) {
			called = true
			c.Status(http.StatusSwitchingProtocols)
		},
	}).router()

	req := httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Connection", "Upgrade")
	req.Header.Set("Upgrade", "websocket")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want 400 body=%s", w.Code, w.Body.String())
	}
	if called {
		t.Fatal("provider gateway must not enter the Codex websocket auth handler")
	}
	if !strings.Contains(w.Body.String(), "websocket_not_supported") {
		t.Fatalf("body = %s", w.Body.String())
	}
}

func TestFilterRegistryModelsByExcludedModels(t *testing.T) {
	models := []*cliproxy.ModelInfo{
		{ID: "gpt-5.3-codex"},
		{ID: "gpt-5.3-codex-spark"},
		{ID: "gpt-5.4"},
	}
	filtered := filterRegistryModelsByExcluded(models, []string{"gpt-5.3-*"})
	if len(filtered) != 1 || filtered[0].ID != "gpt-5.4" {
		t.Fatalf("unexpected filtered models: %#v", filtered)
	}
}

func TestExcludedModelsForAuthMergesManifestAndMetadata(t *testing.T) {
	m := &manifest{
		ExcludedModels: []string{"gpt-image-*"},
		AccountModelRules: []accountModelRule{{
			AccountID:      "plus-account",
			ExcludedModels: []string{"gpt-5.3-codex-spark"},
		}},
		Accounts: []accountSpec{{
			ID:     "plus-account",
			AuthID: "plus-auth",
			Email:  "plus@example.com",
		}},
	}
	m.accountByID = map[string]*accountSpec{"plus-account": &m.Accounts[0]}
	m.accountByAuthID = map[string]*accountSpec{"plus-auth": &m.Accounts[0]}
	auth := &coreauth.Auth{
		ID: "plus-auth",
		Metadata: map[string]any{
			"excluded_models": []string{"custom-model"},
		},
		Attributes: map[string]string{
			"account_id": "plus-account",
		},
	}
	excluded := excludedModelsForAuth(m, auth)
	if len(excluded) != 3 {
		t.Fatalf("expected 3 excluded patterns, got %#v", excluded)
	}
	if !authModelExcluded(m, auth, "gpt-5.3-codex-spark") {
		t.Fatal("spark should be excluded for plus auth")
	}
	if authModelExcluded(m, auth, "gpt-5.4") {
		t.Fatal("gpt-5.4 should remain available")
	}
}

func TestRegisterManifestModelsForAuthRespectsPerAccountExclusions(t *testing.T) {
	m := &manifest{
		ModelIDs: []string{"gpt-5.3-codex", codexSparkModel, "gpt-5.4"},
		Accounts: []accountSpec{{
			ID:     "plus-account",
			AuthID: "plus-auth",
		}},
	}
	m.accountByID = map[string]*accountSpec{"plus-account": &m.Accounts[0]}
	m.accountByAuthID = map[string]*accountSpec{"plus-auth": &m.Accounts[0]}
	auth := &coreauth.Auth{
		ID: "plus-auth",
		Metadata: map[string]any{
			"excluded_models": []string{codexSparkModel},
		},
		Attributes: map[string]string{
			"account_id": "plus-account",
		},
	}
	manager := coreauth.NewManager(nil, &cockpitSelector{manifest: m}, nil)
	t.Cleanup(func() {
		cliproxy.GlobalModelRegistry().UnregisterClient(auth.ID)
	})
	registerManifestModelsForAuth(manager, m, auth)
	models := registry.GetGlobalRegistry().GetModelsForClient(auth.ID)
	for _, model := range models {
		if strings.EqualFold(model.ID, codexSparkModel) {
			t.Fatalf("spark should not be registered for excluded auth: %#v", models)
		}
	}
}

func TestCoreAuthSelectorFiltersNewModelExclusionsBeforeSessionAffinity(t *testing.T) {
	cfg := &config.Config{}
	cfg.Routing.SessionAffinity = true
	cfg.Routing.SessionAffinityTTL = "1h"
	selector := buildCoreAuthSelector(cfg, &coreauth.RoundRobinSelector{}, &manifest{}, nil)
	if stoppable, ok := selector.(coreauth.StoppableSelector); ok {
		t.Cleanup(stoppable.Stop)
	}

	plus := &coreauth.Auth{
		ID:       "plus.json",
		Provider: "codex",
		Status:   coreauth.StatusActive,
		Metadata: map[string]any{},
	}
	pro := &coreauth.Auth{
		ID:       "pro.json",
		Provider: "codex",
		Status:   coreauth.StatusActive,
	}
	auths := []*coreauth.Auth{plus, pro}
	opts := cliproxyexecutor.Options{
		Headers: http.Header{"Session_id": []string{"spark-session"}},
	}

	first, err := selector.Pick(context.Background(), "codex", codexSparkModel, opts, auths)
	if err != nil {
		t.Fatalf("initial Pick: %v", err)
	}
	if first == nil || first.ID != plus.ID {
		t.Fatalf("initial Pick = %#v, want %q", first, plus.ID)
	}

	plus.Metadata["excluded_models"] = []any{codexSparkModel}
	second, err := selector.Pick(context.Background(), "codex", codexSparkModel, opts, auths)
	if err != nil {
		t.Fatalf("Pick after exclusion: %v", err)
	}
	if second == nil || second.ID != pro.ID {
		t.Fatalf("Pick after exclusion = %#v, want %q", second, pro.ID)
	}
}

func TestSidecarRuntimeDoesNotSelectAccountWithExcludedModel(t *testing.T) {
	tempDir := t.TempDir()
	authDir := filepath.Join(tempDir, "auths")
	if err := os.MkdirAll(authDir, 0o755); err != nil {
		t.Fatalf("create auth dir: %v", err)
	}
	configPath := filepath.Join(tempDir, "config.json")
	if err := os.WriteFile(configPath, []byte(`{}`), 0o644); err != nil {
		t.Fatalf("write config path: %v", err)
	}

	blockedFile := "blocked-luna.json"
	allowedFile := "allowed-luna.json"
	if err := os.WriteFile(filepath.Join(authDir, blockedFile), []byte(`{
  "type":"codex",
  "email":"blocked@example.com",
  "access_token":"blocked-token",
  "account_id":"acct-blocked",
  "excluded_models":["GPT-5.6-LUNA", "gpt-5.7-*"]
}`), 0o600); err != nil {
		t.Fatalf("write blocked auth: %v", err)
	}
	if err := os.WriteFile(filepath.Join(authDir, allowedFile), []byte(`{
  "type":"codex",
  "email":"allowed@example.com",
  "access_token":"allowed-token",
  "account_id":"acct-allowed"
}`), 0o600); err != nil {
		t.Fatalf("write allowed auth: %v", err)
	}

	m := &manifest{
		Accounts: []accountSpec{
			{ID: "blocked-account", Email: "blocked@example.com", AuthID: blockedFile, AuthKind: "oauth"},
			{ID: "allowed-account", Email: "allowed@example.com", AuthID: allowedFile, AuthKind: "oauth"},
		},
		ModelIDs:         []string{"gpt-5.6-luna", "gpt-5.7-preview", "gpt-5.4"},
		accountByID:      make(map[string]*accountSpec),
		accountByAuthID:  make(map[string]*accountSpec),
		accountByAPIKey:  make(map[string]*accountSpec),
		accountByChatGPT: make(map[string]*accountSpec),
		accountByEmail:   make(map[string]*accountSpec),
	}
	for index := range m.Accounts {
		account := &m.Accounts[index]
		m.accountByID[account.ID] = account
		m.accountByAuthID[strings.ToLower(account.AuthID)] = account
		m.accountByEmail[strings.ToLower(account.Email)] = account
	}

	cfg := &config.Config{AuthDir: authDir}
	manager := buildCoreAuthManager(cfg, &cockpitSelector{manifest: m}, &authHook{manifest: m}, m, nil, newRequestUsageTracker())
	runtime, err := newSidecarRuntime(context.Background(), configPath, cfg, m, manager)
	if err != nil {
		t.Fatalf("newSidecarRuntime: %v", err)
	}
	defer runtime.Stop()

	for attempt := 0; attempt < 8; attempt++ {
		selected, errSelect := manager.SelectAuth(context.Background(), "codex", "gpt-5.6-luna", cliproxyexecutor.Options{})
		if errSelect != nil {
			t.Fatalf("SelectAuth attempt %d: %v", attempt, errSelect)
		}
		if account := accountForAuthInManifest(m, selected); account == nil || account.ID != "allowed-account" {
			t.Fatalf("SelectAuth attempt %d selected blocked account: auth=%#v account=%#v", attempt, selected, account)
		}
	}

	blockedAuth, ok := manager.GetByID(blockedFile)
	if !ok || blockedAuth == nil {
		t.Fatalf("blocked auth %q was not registered", blockedFile)
	}
	blockedModels := registry.GetGlobalRegistry().GetModelsForClient(blockedAuth.ID)
	if info := findModelInfoForTest(blockedModels, "gpt-5.6-luna"); info != nil {
		t.Fatalf("blocked auth still advertises exact excluded model: %#v", info)
	}
	if info := findModelInfoForTest(blockedModels, "gpt-5.7-preview"); info != nil {
		t.Fatalf("blocked auth still advertises wildcard-excluded model: %#v", info)
	}
	if info := findModelInfoForTest(blockedModels, "gpt-5.4"); info == nil {
		t.Fatal("blocked auth lost a non-excluded model")
	}
}

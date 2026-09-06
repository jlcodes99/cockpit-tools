package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"

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

func TestImageGenerationAllowedForAccountPolicy(t *testing.T) {
	apiKey := &accountSpec{AuthKind: "api_key", ImageGenerationPolicy: "inherit"}
	if imageGenerationAllowedForAccount(apiKey) {
		t.Fatal("API Key accounts should default to image generation disabled")
	}
	apiKey.ImageGenerationPolicy = "enabled"
	if !imageGenerationAllowedForAccount(apiKey) {
		t.Fatal("explicitly enabled API Key account should allow image generation")
	}
	freeOAuth := &accountSpec{AuthKind: "oauth", PlanType: "free", ImageGenerationPolicy: "enabled"}
	if imageGenerationAllowedForAccount(freeOAuth) {
		t.Fatal("free OAuth must not bypass the official image capability")
	}
	plusOAuth := &accountSpec{AuthKind: "oauth", PlanType: "plus", ImageGenerationPolicy: "inherit"}
	if !imageGenerationAllowedForAccount(plusOAuth) {
		t.Fatal("paid OAuth should inherit image generation capability")
	}
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
	if intFromAny(sol["context_window"]) != 272000 || intFromAny(sol["max_context_window"]) != 921000 {
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

func TestCodexClientModelsResponsePreservesAstraTemplate(t *testing.T) {
	response := buildCodexClientModelsResponse([]string{"gpt-6-astra"}, &apiKeySpec{}, nil)
	models, ok := response["models"].([]map[string]any)
	if !ok || len(models) != 1 {
		t.Fatalf("Astra models response = %#v, want one model", response["models"])
	}
	astra := models[0]
	if got := stringFromAny(astra["display_name"]); got != "6 Astra" {
		t.Fatalf("Astra display_name = %q", got)
	}
	if got := intFromAny(astra["context_window"]); got != 1050000 {
		t.Fatalf("Astra context_window = %d, want 1050000", got)
	}
	if got := intFromAny(astra["max_context_window"]); got != 1050000 {
		t.Fatalf("Astra max_context_window = %d, want 1050000", got)
	}
	levels, ok := astra["supported_reasoning_levels"].([]any)
	if !ok {
		t.Fatalf("Astra reasoning levels = %#v", astra["supported_reasoning_levels"])
	}
	for _, effort := range []string{"low", "medium", "high", "xhigh", "max", "ultra"} {
		found := false
		for _, raw := range levels {
			level, _ := raw.(map[string]any)
			if stringFromAny(level["effort"]) == effort {
				found = true
				break
			}
		}
		if !found {
			t.Fatalf("Astra reasoning levels missing %q: %#v", effort, levels)
		}
	}
	if got := stringFromAny(astra["tool_mode"]); got != "code_mode_only" {
		t.Fatalf("Astra tool_mode = %q", got)
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

func TestBuildCockpitQuotaResponseBlocksHourlyWhenWeeklyIsExhausted(t *testing.T) {
	present := true
	fiveHourMinutes := int64(300)
	weeklyMinutes := int64(10080)
	state := quotaPoolStateFile{Accounts: map[string]quotaPoolAccountState{
		"account-1": {
			Primary:   &quotaPoolWindowState{Present: &present, RemainingPercent: intPtrForTest(100), WindowMinutes: &fiveHourMinutes},
			Secondary: &quotaPoolWindowState{Present: &present, RemainingPercent: intPtrForTest(0), WindowMinutes: &weeklyMinutes},
		},
	}}

	response := buildCockpitQuotaResponse(&apiKeySpec{AccountIDs: []string{"account-1"}}, state, time.Now())
	if response.FiveHourRemainingPercent == nil || *response.FiveHourRemainingPercent != 0 {
		t.Fatalf("five-hour percent = %#v, want 0 when weekly is exhausted", response.FiveHourRemainingPercent)
	}
	if response.WeeklyRemainingPercent == nil || *response.WeeklyRemainingPercent != 0 {
		t.Fatalf("weekly percent = %#v, want 0", response.WeeklyRemainingPercent)
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
			name: "deepseek vision model supports vision",
			gateway: &providerGatewaySpec{
				UpstreamModels: []string{"deepseek-v4-flash-vision-exp"},
				ModelCapabilities: map[string]providerGatewayModelCapability{
					"deepseek-v4-flash-vision-exp": {SupportsVision: true},
				},
			},
			model:         "deepseek-v4-flash-vision-exp",
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

	if len(models) != 2 || models[0] != "team/gpt-5.4" || models[1] != "team/gpt-reserve" {
		t.Fatalf("unexpected visible models: %#v", models)
	}
}

func TestCodexReserveListingIsIndependentOfScopedAccountEligibility(t *testing.T) {
	m := &manifest{
		Accounts: []accountSpec{
			{ID: "reserve-account", GPTReserveAllowed: true},
			{ID: "ordinary-account", GPTReserveAllowed: false},
		},
		ModelIDs: []string{codexReserveModel, "gpt-5.4"},
	}
	reserveSpec := &apiKeySpec{AccountIDs: []string{"reserve-account"}}
	ordinarySpec := &apiKeySpec{AccountIDs: []string{"ordinary-account"}}

	if got := visibleModelsForAPIKey(m, reserveSpec); len(got) != 2 || got[0] != codexReserveModel {
		t.Fatalf("entitled API key models = %#v, want reserve first", got)
	}
	if got := visibleModelsForAPIKey(m, ordinarySpec); len(got) != 2 || got[0] != codexReserveModel {
		t.Fatalf("ordinary API key must still list Reserve: %#v", got)
	}
	if !validateClientModelVisible(m, reserveSpec, codexReserveModel, codexReserveModel) {
		t.Fatal("entitled API key should accept gpt-reserve")
	}
	if !validateClientModelVisible(m, ordinarySpec, codexReserveModel, codexReserveModel) {
		t.Fatal("model admission must not hide Reserve; account selection enforces eligibility")
	}
}

func TestCodexReserveClientCatalogListsLunaReserveWithLunaCapabilities(t *testing.T) {
	m := &manifest{
		Accounts: []accountSpec{{ID: "reserve-account", GPTReserveAllowed: true}},
		ModelIDs: []string{codexReserveModel},
	}
	models := clientCatalogModelsForAPIKey(m, &apiKeySpec{AccountIDs: []string{"reserve-account"}})
	response := buildCodexClientModelsResponse(models, &apiKeySpec{}, nil)
	data, ok := response["models"].([]map[string]any)
	if !ok {
		t.Fatalf("models response should contain a models array: %#v", response["models"])
	}
	reserve := findCodexClientModelForTest(data, codexReserveModel)
	if reserve == nil || reserve["display_name"] != "Luna Reserve" || reserve["visibility"] != "list" {
		t.Fatalf("gpt-reserve catalog entry = %#v", reserve)
	}
	lunaResponse := buildCodexClientModelsResponse([]string{"gpt-5.6-luna"}, &apiKeySpec{}, nil)
	luna := findCodexClientModelForTest(lunaResponse["models"].([]map[string]any), "gpt-5.6-luna")
	for _, field := range []string{"context_window", "max_context_window", "auto_compact_token_limit", "supported_reasoning_levels", "default_reasoning_level", "input_modalities", "tool_mode", "shell_type", "use_responses_lite", "priority"} {
		if !reflect.DeepEqual(reserve[field], luna[field]) {
			t.Fatalf("Reserve %s = %#v, want Luna value %#v", field, reserve[field], luna[field])
		}
	}
	if reserve["auto_compact_token_limit"] != nil {
		t.Fatal("Reserve must not force a compaction threshold")
	}
	info := manifestRegistryModelInfo(codexReserveModel, "", 0)
	if !reflect.DeepEqual(info.Thinking, codexClientThinkingSupport("gpt-5.6-luna")) || info.Thinking == nil {
		t.Fatalf("Reserve request reasoning capabilities = %#v", info.Thinking)
	}
}

func TestReserveModelAdmissionKeepsIDAndStillHonorsExplicitAccessFilters(t *testing.T) {
	m := &manifest{ModelIDs: []string{codexReserveModel, "gpt-5.6-luna"}}
	spec := &apiKeySpec{AccountIDs: []string{"no-eligible-account"}}
	body := []byte(`{"model":"gpt-reserve","input":"hello"}`)
	rewritten, model, err := rewriteBodyModel(m, spec, body)
	if err != nil || rewritten != nil || model != codexReserveModel {
		t.Fatalf("Reserve must keep the original request unchanged: model=%q, err=%v", model, err)
	}
	spec.ExcludedModels = []string{codexReserveModel}
	if validateClientModelVisible(m, spec, codexReserveModel, codexReserveModel) {
		t.Fatal("persistent listing must not bypass an explicit API key model exclusion")
	}
	spec.ExcludedModels = nil
	spec.AllowedModels = []string{"gpt-5.6-luna"}
	if validateClientModelVisible(m, spec, codexReserveModel, codexReserveModel) {
		t.Fatal("persistent listing must not bypass an explicit API key model allowlist")
	}
}

func TestPrefixedCodexReserveKeepsVisibleLunaCapabilitiesAndExplicitContext(t *testing.T) {
	spec := &apiKeySpec{ModelPrefix: "team"}
	response := buildCodexClientModelsResponse([]string{"team/gpt-reserve"}, spec, map[string]int64{"gpt-reserve": 516000})
	reserve := findCodexClientModelForTest(response["models"].([]map[string]any), "team/gpt-reserve")
	if reserve == nil || reserve["visibility"] != "list" || reserve["display_name"] != "Luna Reserve" {
		t.Fatalf("prefixed Reserve = %#v", reserve)
	}
	if intFromAny(reserve["context_window"]) != 516000 || reserve["auto_compact_token_limit"] != nil {
		t.Fatalf("explicit Reserve context must not set compaction: %#v", reserve)
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

	if len(models) != 3 || models[0] != "team/gpt-5.4" || models[1] != "team/gpt-reserve" || models[2] != codexAutoReviewModel {
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

func TestCockpitSelectorUsesOnlyAccountsWithCodexReserveEntitlement(t *testing.T) {
	highPlanOrdinary := &accountSpec{
		ID:                "ordinary-account",
		AuthID:            "ordinary-account.json",
		PlanRank:          intPtrForTest(500),
		GPTReserveAllowed: false,
	}
	reserveAccount := &accountSpec{
		ID:                "reserve-account",
		AuthID:            "reserve-account.json",
		PlanRank:          intPtrForTest(300),
		GPTReserveAllowed: true,
	}
	selector := &cockpitSelector{
		manifest: &manifest{
			RoutingStrategy: "auto",
			Accounts:        []accountSpec{*highPlanOrdinary, *reserveAccount},
			accountByAuthID: map[string]*accountSpec{
				"ordinary-account.json": highPlanOrdinary,
				"reserve-account.json":  reserveAccount,
			},
			accountByID: map[string]*accountSpec{
				"ordinary-account": highPlanOrdinary,
				"reserve-account":  reserveAccount,
			},
		},
	}
	auths := []*coreauth.Auth{
		{ID: "ordinary-account.json", Provider: "codex", Status: coreauth.StatusActive},
		{ID: "reserve-account.json", Provider: "codex", Status: coreauth.StatusActive},
	}

	selected, err := selector.Pick(context.Background(), "codex", codexReserveModel, cliproxyexecutor.Options{}, auths)

	if err != nil {
		t.Fatalf("pick gpt-reserve auth: %v", err)
	}
	if selected == nil || selected.ID != "reserve-account.json" {
		t.Fatalf("selected auth = %#v, want entitled account", selected)
	}
	ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, &apiKeySpec{
		ID: "ordinary-only", AccountIDs: []string{"ordinary-account"},
	})
	selected, err = selector.Pick(ctx, "codex", codexReserveModel, cliproxyexecutor.Options{}, auths)
	if err == nil || selected != nil {
		t.Fatal("must not use an out-of-scope entitled account or an ineligible scoped account")
	}
	selected, err = selector.Pick(context.Background(), "codex", codexReserveModel, cliproxyexecutor.Options{}, auths[:1])
	if err == nil || selected != nil {
		t.Fatal("an ordinary-only pool must fail instead of routing Reserve to an ordinary account")
	}
}

func TestAccountPoolFailureEmitsScopeDiagnosticsWithoutAccountID(t *testing.T) {
	account := &accountSpec{ID: "account-1", AuthID: "account-1.json"}
	selector := &cockpitSelector{
		manifest: &manifest{
			accountByAuthID: map[string]*accountSpec{"account-1.json": account},
			accountByID:     map[string]*accountSpec{"account-1": account},
		},
		emitter: &eventEmitter{},
	}
	apiKey := &apiKeySpec{
		ID:         "key-scoped",
		Label:      "Scoped client",
		AccountIDs: []string{"missing-account"},
	}
	ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, apiKey)
	ctx = context.WithValue(ctx, requestKindContextKey, "text")
	auths := []*coreauth.Auth{{ID: "account-1.json", Provider: "codex", Status: coreauth.StatusActive}}

	out := captureStdout(t, func() {
		if _, err := selector.Pick(ctx, "codex", "gpt-5.5", cliproxyexecutor.Options{}, auths); err == nil {
			t.Fatal("expected selection to fail when API key scope matches no account")
		}
	})

	var payload requestDiagnosticPayload
	if err := json.Unmarshal([]byte(out), &payload); err != nil {
		t.Fatalf("pool diagnostic should be JSON: %v\n%s", err, out)
	}
	if payload.Type != "auth_pool_result" || payload.APIKeyID != "key-scoped" {
		t.Fatalf("unexpected pool diagnostic identity: %#v", payload)
	}
	if payload.AccountID != "" || payload.CandidateAuths != 1 || payload.ScopedAuths != 0 || payload.AvailableAuths != 0 {
		t.Fatalf("pool diagnostic should explain pre-account scope exhaustion: %#v", payload)
	}
}

func TestAccountPoolFailureIncludesPerAccountUnavailableReason(t *testing.T) {
	account := &accountSpec{ID: "account-1", Email: "one@example.com", AuthID: "account-1.json"}
	selector := &cockpitSelector{
		manifest: &manifest{
			accountByAuthID: map[string]*accountSpec{"account-1.json": account},
			accountByID:     map[string]*accountSpec{"account-1": account},
		},
		emitter: &eventEmitter{},
	}
	auths := []*coreauth.Auth{{
		ID:        "account-1.json",
		Provider:  "codex",
		Status:    coreauth.StatusDisabled,
		LastError: &coreauth.Error{Code: "invalid_refresh_token", Message: "refresh token is invalid"},
	}}

	out := captureStdout(t, func() {
		if _, err := selector.Pick(context.Background(), "codex", "gpt-5.5", cliproxyexecutor.Options{}, auths); err == nil {
			t.Fatal("expected unavailable auth selection to fail")
		}
	})

	var payload requestDiagnosticPayload
	if err := json.Unmarshal([]byte(out), &payload); err != nil {
		t.Fatalf("pool diagnostic should be JSON: %v\n%s", err, out)
	}
	if len(payload.AccountStatuses) != 1 {
		t.Fatalf("expected one account diagnostic, got %#v; raw=%s", payload.AccountStatuses, out)
	}
	status := payload.AccountStatuses[0]
	if status.AccountID != "account-1" || status.AccountEmail != "one@example.com" || status.Available {
		t.Fatalf("unexpected account identity/status: %#v", status)
	}
	if status.ReasonCode != "disabled" || !strings.Contains(status.ReasonMessage, "account is disabled") {
		t.Fatalf("unexpected unavailable reason: %#v", status)
	}
}

func TestAuthPoolUnavailableErrorUsesCockpitLocale(t *testing.T) {
	stats := authPoolSelectionStats{
		candidateAuths:     3,
		unavailableAuths:   1,
		modelExcludedAuths: 1,
		quotaReservedAuths: 1,
	}

	zhError := authPoolUnavailableError("zh-CN", stats, "no auth available")
	if !strings.Contains(zhError.Message, "账号池没有可用账号：候选 3 个") {
		t.Fatalf("unexpected Chinese error message: %q", zhError.Message)
	}

	englishError := authPoolUnavailableError("ja", stats, "no auth available")
	if !strings.Contains(englishError.Message, "No available account: candidates=3") {
		t.Fatalf("unexpected English error message: %q", englishError.Message)
	}
}

func TestCockpitLocaleIgnoresRequestAcceptLanguage(t *testing.T) {
	recorder := httptest.NewRecorder()
	ginContext, _ := gin.CreateTestContext(recorder)
	ginContext.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", nil)
	ginContext.Request.Header.Set("Accept-Language", "zh-CN,zh;q=0.9")

	selector := &cockpitSelector{locale: "en-US"}
	_, err := selector.Pick(
		context.WithValue(context.Background(), "gin", ginContext),
		"codex",
		"gpt-5.5",
		cliproxyexecutor.Options{},
		nil,
	)
	if err == nil || !strings.Contains(err.Error(), "No available account") {
		t.Fatalf("request language must not override Cockpit locale: %v", err)
	}
}

func TestManagerAvailabilityFailureProducesPoolDiagnostics(t *testing.T) {
	account := &accountSpec{ID: "account-1", Email: "one@example.com", AuthID: "account-1.json"}
	selector := &cockpitSelector{
		manifest: &manifest{
			accountByAuthID: map[string]*accountSpec{"account-1.json": account},
			accountByID:     map[string]*accountSpec{"account-1": account},
		},
		emitter: &eventEmitter{},
	}
	auth := &coreauth.Auth{
		ID:             "account-1.json",
		Provider:       "codex",
		Status:         coreauth.StatusActive,
		Unavailable:    true,
		NextRetryAfter: time.Now().Add(time.Hour),
		LastError:      &coreauth.Error{Code: "invalid_refresh_token", Message: "refresh token is invalid"},
	}

	var reportedErr error
	out := captureStdout(t, func() {
		reportedErr = selector.ReportAuthSelectionFailure(
			context.Background(),
			"codex",
			"gpt-5.5",
			[]*coreauth.Auth{auth},
			&coreauth.Error{Code: "auth_unavailable", Message: "no auth available"},
		)
	})

	var authErr *coreauth.Error
	if !errors.As(reportedErr, &authErr) || authErr == nil {
		t.Fatalf("expected detailed auth error, got %T %v", reportedErr, reportedErr)
	}
	if !strings.Contains(authErr.Message, "No available account: candidates=1, unavailable=1") {
		t.Fatalf("error should contain account-pool statistics: %q", authErr.Message)
	}
	var payload requestDiagnosticPayload
	if err := json.Unmarshal([]byte(out), &payload); err != nil {
		t.Fatalf("pool diagnostic should be JSON: %v\n%s", err, out)
	}
	if payload.Type != "auth_pool_result" || payload.CandidateAuths != 1 || payload.UnavailableAuths != 1 {
		t.Fatalf("unexpected manager-level pool diagnostic: %#v", payload)
	}
	if len(payload.AccountStatuses) != 1 || payload.AccountStatuses[0].AccountID != "account-1" {
		t.Fatalf("missing account-level manager diagnostic: %#v", payload.AccountStatuses)
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

func TestManifestRegistryModelsPreservesAstraThinkingSupport(t *testing.T) {
	models := manifestRegistryModels(&manifest{
		ModelIDs: []string{"gpt-6-astra"},
	})
	info := findModelInfoForTest(models, "gpt-6-astra")
	if info == nil {
		t.Fatal("expected gpt-6-astra in manifest registry models")
	}
	if info.Thinking == nil {
		t.Fatalf("Astra thinking support is missing: %#v", info)
	}
	for _, effort := range []string{"low", "medium", "high", "xhigh", "max", "ultra"} {
		found := false
		for _, level := range info.Thinking.Levels {
			if level == effort {
				found = true
				break
			}
		}
		if !found {
			t.Fatalf("Astra thinking levels missing %q: %#v", effort, info.Thinking.Levels)
		}
	}
	if info.UserDefined {
		t.Fatalf("Astra should use shipped static capabilities: %#v", info)
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
	if payload.Model != "gpt-5.5" || payload.AccountID != "account_1" || payload.APIKeyID != "key_1" {
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

func TestResolveModelRoutingSeparatesOAuthAndProviderModels(t *testing.T) {
	gateway := &providerGatewaySpec{
		BaseURL:        "https://provider.example/v1",
		APIKey:         "secret",
		UpstreamModels: []string{"gpt-5.5", "grok-4.6"},
	}
	spec := &apiKeySpec{ModelRouting: &modelRoutingSpec{
		DefaultRoute:  "oauth",
		FailurePolicy: "strict",
		Routes: []modelRouteSpec{{
			ID:              "route-cpa",
			Namespace:       "cpa",
			ProviderGateway: gateway,
		}},
	}}

	if gotGateway, gotModel, status := resolveModelRouting(spec, "gpt-5.5"); gotGateway != nil || gotModel != "gpt-5.5" || status != "none" {
		t.Fatalf("bare model should stay on OAuth: gateway=%v model=%q status=%q", gotGateway, gotModel, status)
	}
	if gotGateway, gotModel, status := resolveModelRouting(spec, "cpa/gpt-5.5"); gotGateway != gateway || gotModel != "gpt-5.5" || status != "matched" {
		t.Fatalf("namespaced GPT should use provider: gateway=%v model=%q status=%q", gotGateway, gotModel, status)
	}
	if gotGateway, gotModel, status := resolveModelRouting(spec, "cpa/grok-4.6"); gotGateway != gateway || gotModel != "grok-4.6" || status != "matched" {
		t.Fatalf("namespaced provider model should use provider: gateway=%v model=%q status=%q", gotGateway, gotModel, status)
	}
}

func TestResolveModelRoutingNeverFallsBackForUnknownNamespaceOrModel(t *testing.T) {
	spec := &apiKeySpec{ModelRouting: &modelRoutingSpec{
		DefaultRoute:  "oauth",
		FailurePolicy: "strict",
		Routes: []modelRouteSpec{{
			ID:        "route-cpa",
			Namespace: "cpa",
			ProviderGateway: &providerGatewaySpec{
				BaseURL:        "https://provider.example/v1",
				APIKey:         "secret",
				UpstreamModels: []string{"gpt-5.5"},
			},
		}},
	}}

	for _, model := range []string{"missing/gpt-5.5", "cpa/grok-4.6", "cpa/"} {
		if gateway, upstream, status := resolveModelRouting(spec, model); gateway != nil || upstream != "" || status != "missing" {
			t.Fatalf("%s should fail strictly: gateway=%v model=%q status=%q", model, gateway, upstream, status)
		}
	}
}

func TestResolveModelRoutingEmptyCatalogRejectsNamespacedModel(t *testing.T) {
	spec := &apiKeySpec{ModelRouting: &modelRoutingSpec{
		DefaultRoute:  "oauth",
		FailurePolicy: "strict",
		Routes: []modelRouteSpec{{
			ID:        "route-cpa",
			Namespace: "cpa",
			ProviderGateway: &providerGatewaySpec{
				BaseURL: "https://provider.example/v1",
				APIKey:  "secret",
			},
		}},
	}}

	gateway, upstream, status := resolveModelRouting(spec, "cpa/gpt-5.5")
	if status != "missing" || gateway != nil || upstream != "" {
		t.Fatalf("empty catalog should reject routing: gateway=%v model=%q status=%q", gateway, upstream, status)
	}
}

func TestResolveModelRoutingRejectsRouteWithoutProviderGateway(t *testing.T) {
	spec := &apiKeySpec{ModelRouting: &modelRoutingSpec{
		DefaultRoute:  "oauth",
		FailurePolicy: "strict",
		Routes: []modelRouteSpec{{
			ID:        "route-cpa",
			Namespace: "cpa",
		}},
	}}

	if gateway, upstream, status := resolveModelRouting(spec, "cpa/gpt-5.5"); gateway != nil || upstream != "" || status != "missing" {
		t.Fatalf("route without provider gateway should fail strictly: gateway=%v model=%q status=%q", gateway, upstream, status)
	}
}

func TestMixedRoutingCatalogPreservesGPTCapabilities(t *testing.T) {
	spec := mixedRoutingAPIKey(&providerGatewaySpec{
		UpstreamModels: []string{"gpt-6-astra"},
		WireAPI: "responses",
	})
	catalog := buildCodexClientModelsResponse([]string{"gpt-6-astra", "cpa/gpt-6-astra"}, spec, nil)
	models := catalog["models"].([]map[string]any)
	if models[1]["slug"] != "cpa/gpt-6-astra" { t.Fatal("lost routing identity") }
	for _, field := range []string{"service_tiers", "additional_speed_tiers", "supported_reasoning_levels", "context_window"} {
		a, _ := json.Marshal(models[0][field])
		b, _ := json.Marshal(models[1][field])
		if string(a) != string(b) { t.Fatalf("%s differs: %s != %s", field, a, b) }
	}
}

func TestVisibleModelsForMixedRoutingIncludesNamespacedProviderModels(t *testing.T) {
	m := &manifest{ModelIDs: []string{"gpt-5.5"}}
	spec := &apiKeySpec{ModelRouting: &modelRoutingSpec{
		DefaultRoute:  "oauth",
		FailurePolicy: "strict",
		Routes: []modelRouteSpec{{
			ID:        "route-cpa",
			Namespace: "cpa",
			ProviderGateway: &providerGatewaySpec{
				UpstreamModels: []string{"gpt-5.5", "grok-4.6"},
			},
		}},
	}}

	got := visibleModelsForAPIKey(m, spec)
	want := []string{"gpt-5.5", "gpt-reserve", "cpa/gpt-5.5", "cpa/grok-4.6"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("visible models = %#v, want %#v", got, want)
	}
}

func TestAccountQuotaExhaustedOnlyBlocksKnownOAuthZeroQuota(t *testing.T) {
	zero := 0
	positive := 12
	if !accountQuotaExhausted(nil, &accountSpec{AuthKind: "oauth", RemainingQuota: &zero}, time.Now()) {
		t.Fatal("known zero OAuth quota should be exhausted")
	}
	if accountQuotaExhausted(nil, &accountSpec{AuthKind: "oauth", RemainingQuota: &positive}, time.Now()) {
		t.Fatal("positive OAuth quota should remain available")
	}
	if accountQuotaExhausted(nil, &accountSpec{AuthKind: "oauth"}, time.Now()) {
		t.Fatal("unknown OAuth quota should not be treated as exhausted")
	}
	if accountQuotaExhausted(nil, &accountSpec{AuthKind: "api_key", RemainingQuota: &zero}, time.Now()) {
		t.Fatal("API-key accounts must use their own quota policy")
	}
}

func TestCockpitSelectorSkipsExhaustedQuotaForRegularModels(t *testing.T) {
	zero := 0
	account := &accountSpec{ID: "account-1", AuthKind: "oauth", RemainingQuota: &zero, GPTReserveAllowed: true}
	auth := &coreauth.Auth{ID: "auth-1", Provider: "codex"}
	m := &manifest{
		ModelIDs:       []string{"gpt-5.5", codexReserveModel},
		Accounts:       []accountSpec{*account},
		accountByAuthID: map[string]*accountSpec{"auth-1": account},
	}
	selector := &cockpitSelector{manifest: m}

	selected, err := selector.Pick(context.Background(), "codex", "gpt-5.5", cliproxyexecutor.Options{}, []*coreauth.Auth{auth})
	if selected != nil || err == nil {
		t.Fatalf("regular model should be blocked by zero quota: selected=%v err=%v", selected, err)
	}

	selected, err = selector.Pick(context.Background(), "codex", codexReserveModel, cliproxyexecutor.Options{}, []*coreauth.Auth{auth})
	if err != nil || selected != auth {
		t.Fatalf("reserve model should keep its independent quota path: selected=%v err=%v", selected, err)
	}
}

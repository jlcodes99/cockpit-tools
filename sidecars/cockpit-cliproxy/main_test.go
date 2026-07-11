package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
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

func TestCodexClientModelsResponseShape(t *testing.T) {
	response := buildCodexClientModelsResponse([]string{"gpt-5.4", "gpt-image-2", codexAutoReviewModel})
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
	if _, ok := textModel["prefer_websockets"].(bool); !ok {
		t.Fatalf("text model should keep websocket preference: %#v", textModel)
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
	if imageModel["visibility"] != "hide" {
		t.Fatalf("image model should be hidden in Codex client catalog: %#v", imageModel)
	}
	if reviewModel["visibility"] != "hide" {
		t.Fatalf("auto review model should be hidden in Codex client catalog: %#v", reviewModel)
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
	manager := buildCoreAuthManager(cfg, &cockpitSelector{manifest: m}, &authHook{manifest: m}, m, nil)

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
	manager := buildCoreAuthManager(&config.Config{}, &cockpitSelector{}, nil, nil, nil)
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
			AuthID:   "auth.json",
			Provider: "codex",
			Model:    "upstream-model",
			Success:  false,
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

func TestRelayServerProviderGatewayRejectsVisionInputWhenUnsupported(t *testing.T) {
	gin.SetMode(gin.TestMode)
	upstreamCalled := false
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		upstreamCalled = true
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

	if w.Code != http.StatusBadRequest {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if upstreamCalled {
		t.Fatal("unsupported image input without routing model should not call upstream")
	}
	if !strings.Contains(w.Body.String(), "unsupported_image_input") {
		t.Fatalf("unsupported image input should return explicit error: %s", w.Body.String())
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
}

func (r *fakeRuntime) Execute(_ context.Context, _ []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	r.executeCalls++
	r.lastReq = req
	r.lastOpts = opts
	return r.response, r.err
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

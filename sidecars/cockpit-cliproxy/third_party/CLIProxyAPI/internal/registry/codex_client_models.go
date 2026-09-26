package registry

import (
	"bytes"
	_ "embed"
	"encoding/json"
	"fmt"
	"math"
	"strings"
	"sync"

	log "github.com/sirupsen/logrus"
)

//go:embed models/codex_client_models.json
var embeddedCodexClientModelsJSON []byte

type codexClientModelsPayload struct {
	Models []map[string]any `json:"models"`
}

// locallyPinnedCodexClientModelSlugs lists the shipped Codex client models that
// must stay in the catalog even when a remote registry lags behind.
var locallyPinnedCodexClientModelSlugs = []string{
	codexBuiltinGPT6AstraModelID,
	codexBuiltinGPT6SolModelID,
	codexBuiltinGPT6LunaModelID,
}

// mergeLocallyPinnedCodexClientModels keeps shipped models available when a
// remote registry has not learned them yet. Remote metadata wins once present.
func mergeLocallyPinnedCodexClientModels(data []byte) ([]byte, error) {
	var document map[string]json.RawMessage
	if err := json.Unmarshal(data, &document); err != nil {
		return nil, fmt.Errorf("decode remote Codex client model catalog: %w", err)
	}
	modelsRaw, ok := document["models"]
	if !ok {
		return nil, fmt.Errorf("remote Codex client model catalog is missing models")
	}
	var models []map[string]any
	if err := json.Unmarshal(modelsRaw, &models); err != nil {
		return nil, fmt.Errorf("decode remote Codex client models: %w", err)
	}

	presentSlugs := make(map[string]struct{}, len(models))
	for _, model := range models {
		presentSlugs[strings.ToLower(strings.TrimSpace(fmt.Sprint(model["slug"])))] = struct{}{}
	}
	missingSlugs := make([]string, 0, len(locallyPinnedCodexClientModelSlugs))
	for _, slug := range locallyPinnedCodexClientModelSlugs {
		if _, ok := presentSlugs[strings.ToLower(strings.TrimSpace(slug))]; !ok {
			missingSlugs = append(missingSlugs, slug)
		}
	}
	if len(missingSlugs) == 0 {
		return append([]byte(nil), data...), nil
	}

	var embedded codexClientModelsPayload
	if err := json.Unmarshal(embeddedCodexClientModelsJSON, &embedded); err != nil {
		return nil, fmt.Errorf("decode embedded Codex client model catalog: %w", err)
	}
	embeddedBySlug := make(map[string]map[string]any, len(embedded.Models))
	for _, model := range embedded.Models {
		embeddedBySlug[strings.ToLower(strings.TrimSpace(fmt.Sprint(model["slug"])))] = model
	}
	for _, slug := range missingSlugs {
		model, ok := embeddedBySlug[strings.ToLower(strings.TrimSpace(slug))]
		if !ok {
			return nil, fmt.Errorf("embedded Codex client catalog is missing locally pinned models %v", missingSlugs)
		}
		models = append(models, model)
	}

	mergedModels, err := json.Marshal(models)
	if err != nil {
		return nil, fmt.Errorf("encode merged Codex client models: %w", err)
	}
	document["models"] = mergedModels
	return json.Marshal(document)
}

type codexClientModelsStore struct {
	mu       sync.RWMutex
	data     []byte
	revision uint64
}

var codexClientCatalogStore = &codexClientModelsStore{}

func init() {
	if _, err := loadCodexClientModelsFromBytes(embeddedCodexClientModelsJSON, "embed"); err != nil {
		log.Warnf("registry: failed to parse embedded codex_client_models.json (Codex client catalog will remain unavailable until a valid remote refresh): %v", err)
	}
}

// GetCodexClientModelsJSON returns the current Codex client model catalog.
func GetCodexClientModelsJSON() []byte {
	data, _ := GetCodexClientModelsSnapshot()
	return data
}

func CodexClientModelBaseInstructions(modelID string) string {
	var payload codexClientModelsPayload
	if json.Unmarshal(GetCodexClientModelsJSON(), &payload) != nil {
		return ""
	}
	for _, model := range payload.Models {
		if strings.EqualFold(strings.TrimSpace(fmt.Sprint(model["slug"])), strings.TrimSpace(modelID)) {
			return strings.TrimSpace(fmt.Sprint(model["base_instructions"]))
		}
	}
	return ""
}

// CodexClientModelUsesResponsesLite reports whether the active Codex client
// catalog routes the model through the Responses Lite protocol.
func CodexClientModelUsesResponsesLite(modelID string) bool {
	modelID = strings.TrimSpace(modelID)
	if modelID == "" {
		return false
	}
	var payload codexClientModelsPayload
	if json.Unmarshal(GetCodexClientModelsJSON(), &payload) != nil {
		return false
	}
	for _, model := range payload.Models {
		if !strings.EqualFold(strings.TrimSpace(fmt.Sprint(model["slug"])), modelID) {
			continue
		}
		value, _ := model["use_responses_lite"].(bool)
		return value
	}
	return false
}

// GetCodexClientModelsRevision returns the current revision of the Codex client model catalog.
func GetCodexClientModelsRevision() uint64 {
	codexClientCatalogStore.mu.RLock()
	defer codexClientCatalogStore.mu.RUnlock()
	return codexClientCatalogStore.revision
}

// GetCodexClientModelsSnapshot returns a consistent catalog copy and revision.
// The revision changes only when validated catalog content changes.
func GetCodexClientModelsSnapshot() ([]byte, uint64) {
	codexClientCatalogStore.mu.RLock()
	defer codexClientCatalogStore.mu.RUnlock()
	return append([]byte(nil), codexClientCatalogStore.data...), codexClientCatalogStore.revision
}

func loadCodexClientModelsFromBytes(data []byte, source string) (bool, error) {
	if err := ValidateCodexClientModelsJSON(data); err != nil {
		return false, fmt.Errorf("%s: %w", source, err)
	}

	cloned := append([]byte(nil), data...)
	codexClientCatalogStore.mu.Lock()
	defer codexClientCatalogStore.mu.Unlock()
	if bytes.Equal(codexClientCatalogStore.data, cloned) {
		return false, nil
	}
	codexClientCatalogStore.data = cloned
	codexClientCatalogStore.revision++
	return true, nil
}

// ValidateCodexClientModelsJSON validates the fields required to serve a
// complete Codex client model catalog.
func ValidateCodexClientModelsJSON(data []byte) error {
	var payload codexClientModelsPayload
	if err := json.Unmarshal(data, &payload); err != nil {
		return fmt.Errorf("decode Codex client model catalog: %w", err)
	}
	if len(payload.Models) == 0 {
		return fmt.Errorf("Codex client model catalog has no models")
	}

	seen := make(map[string]struct{}, len(payload.Models))
	for i, model := range payload.Models {
		slug, err := requiredCodexClientModelString(model, "slug")
		if err != nil {
			return fmt.Errorf("Codex client model catalog models[%d]: %w", i, err)
		}
		if _, exists := seen[slug]; exists {
			return fmt.Errorf("Codex client model catalog contains duplicate slug %q", slug)
		}
		seen[slug] = struct{}{}

		if err = validateCodexClientModel(model); err != nil {
			return fmt.Errorf("Codex client model catalog model %q: %w", slug, err)
		}
	}
	if _, ok := seen["gpt-5.5"]; !ok {
		return fmt.Errorf("Codex client model catalog is missing default template %q", "gpt-5.5")
	}
	return nil
}

func validateCodexClientModel(model map[string]any) error {
	for _, field := range []string{
		"display_name",
		"description",
		"base_instructions",
		"minimal_client_version",
		"visibility",
		"default_reasoning_level",
	} {
		if _, err := requiredCodexClientModelString(model, field); err != nil {
			return err
		}
	}

	contextWindow, err := requiredCodexClientModelInteger(model, "context_window", true)
	if err != nil {
		return err
	}
	maxContextWindow, err := requiredCodexClientModelInteger(model, "max_context_window", true)
	if err != nil {
		return err
	}
	if contextWindow > maxContextWindow {
		return fmt.Errorf("context_window %d exceeds max_context_window %d", contextWindow, maxContextWindow)
	}
	if _, err = requiredCodexClientModelInteger(model, "priority", false); err != nil {
		return err
	}

	levels, ok := model["supported_reasoning_levels"].([]any)
	if !ok || len(levels) == 0 {
		return fmt.Errorf("field %q must be a non-empty array", "supported_reasoning_levels")
	}
	seenLevels := make(map[string]struct{}, len(levels))
	for i, rawLevel := range levels {
		level, ok := rawLevel.(map[string]any)
		if !ok {
			return fmt.Errorf("field %q entry %d must be an object", "supported_reasoning_levels", i)
		}
		effort, errEffort := requiredCodexClientModelString(level, "effort")
		if errEffort != nil {
			return fmt.Errorf("field %q entry %d: %w", "supported_reasoning_levels", i, errEffort)
		}
		if _, exists := seenLevels[effort]; exists {
			return fmt.Errorf("field %q contains duplicate effort %q", "supported_reasoning_levels", effort)
		}
		seenLevels[effort] = struct{}{}
	}
	defaultLevel, _ := requiredCodexClientModelString(model, "default_reasoning_level")
	if _, ok = seenLevels[defaultLevel]; !ok {
		return fmt.Errorf("default_reasoning_level %q is not listed in supported_reasoning_levels", defaultLevel)
	}
	return nil
}

func requiredCodexClientModelString(model map[string]any, field string) (string, error) {
	value, ok := model[field].(string)
	value = strings.TrimSpace(value)
	if !ok || value == "" {
		return "", fmt.Errorf("field %q must be a non-empty string", field)
	}
	return value, nil
}

func requiredCodexClientModelInteger(model map[string]any, field string, positive bool) (int64, error) {
	value, ok := model[field].(float64)
	if !ok || math.IsNaN(value) || math.IsInf(value, 0) || math.Trunc(value) != value || value > math.MaxInt64 {
		return 0, fmt.Errorf("field %q must be an integer", field)
	}
	if positive && value <= 0 {
		return 0, fmt.Errorf("field %q must be positive", field)
	}
	if !positive && value < 0 {
		return 0, fmt.Errorf("field %q must not be negative", field)
	}
	return int64(value), nil
}

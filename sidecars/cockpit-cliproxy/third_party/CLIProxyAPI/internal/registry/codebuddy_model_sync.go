package registry

import (
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"sort"
	"strings"
	"sync"
	"time"

	log "github.com/sirupsen/logrus"
)

const (
	// codebuddyModelSyncInterval controls how often the codebuddy model list is
	// re-synced from the locally installed official WorkBuddy/CodeBuddy client.
	codebuddyModelSyncInterval = 3 * time.Hour

	// codebuddyModelCreatedAt is the fallback "created" timestamp assigned to
	// models extracted from the official client (which does not publish one).
	// It matches the existing static codebuddy entries.
	codebuddyModelCreatedAt = 1752192000
)

// codebuddyAsarModelRe matches the inline model definition literals embedded in
// the official client's app.asar. Each entry carries a provider group id, a
// concrete model id, a human-readable display name and capability flags
// (supportsTools / supportsImages). supportsImages drives the vision-capability
// metadata used by the vision-proxy routing (text-only models that receive
// image input are transparently pre-processed by a vision model).
var codebuddyAsarModelRe = regexp.MustCompile(
	`providerId\s*:\s*"([^"]+)"\s*,\s*model\s*:\s*"([^"]+)"\s*,\s*displayName\s*:\s*"([^"]+)"\s*,\s*supportsTools\s*:\s*(true|false)\s*,\s*supportsImages\s*:\s*(true|false)`,
)

// codebuddySyncMu guards codebuddySynced.
var codebuddySyncMu sync.RWMutex

// codebuddySynced holds the model list extracted from the local official client.
// nil means a successful extraction has not happened yet (fall back to static).
var codebuddySynced []*ModelInfo

// runCodebuddyModelSync periodically re-reads the locally installed official
// WorkBuddy/CodeBuddy client's app.asar and keeps the codebuddy model catalog
// in sync with whatever models the official client currently advertises.
//
// Unlike the remote models.json refresh, the official Tencent CodeBuddy backend
// exposes no public model-list endpoint, so the client bundle is the only
// authoritative local source of the real model list.
func runCodebuddyModelSync(ctx context.Context) {
	tryCodebuddySync("startup codebuddy model sync")
	ticker := time.NewTicker(codebuddyModelSyncInterval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			tryCodebuddySync("periodic codebuddy model sync")
		}
	}
}

func tryCodebuddySync(label string) {
	asarPath := findCodebuddyAsarPath()
	if asarPath == "" {
		log.Debugf("%s: no official WorkBuddy/CodeBuddy client found, keeping static codebuddy models", label)
		return
	}

	models, err := extractCodebuddyModels(asarPath)
	if err != nil {
		log.Warnf("%s: failed to extract models from %s: %v", label, asarPath, err)
		return
	}
	if len(models) == 0 {
		log.Warnf("%s: extracted 0 models from %s", label, asarPath)
		return
	}

	codebuddySyncMu.Lock()
	changed := modelSectionChanged(codebuddySynced, models)
	codebuddySynced = models
	codebuddySyncMu.Unlock()

	if changed {
		log.Infof("%s: codebuddy model list updated from %s (%d models)", label, asarPath, len(models))
		notifyModelRefresh([]string{"codebuddy"})
		return
	}
	log.Infof("%s: no codebuddy model changes (%d models)", label, len(models))
}

// findCodebuddyAsarPath locates the official client bundle on the current
// platform. Returns an empty string when the client is not installed.
func findCodebuddyAsarPath() string {
	for _, p := range codebuddyAsarCandidates() {
		if p == "" {
			continue
		}
		if st, err := os.Stat(p); err == nil && !st.IsDir() {
			return p
		}
	}
	return ""
}

// codebuddyAsarCandidates returns the candidate app.asar locations for the
// current OS, ordered by likelihood.
func codebuddyAsarCandidates() []string {
	var paths []string
	switch runtime.GOOS {
	case "windows":
		if local := os.Getenv("LOCALAPPDATA"); local != "" {
			paths = append(paths,
				filepath.Join(local, "Programs", "WorkBuddy", "resources", "app.asar"),
				filepath.Join(local, "Programs", "workbuddy", "resources", "app.asar"),
				filepath.Join(local, "Programs", "CodeBuddy", "resources", "app.asar"),
				filepath.Join(local, "Programs", "codebuddy", "resources", "app.asar"),
			)
		}
	case "darwin":
		paths = append(paths,
			"/Applications/WorkBuddy.app/Contents/Resources/app.asar",
			"/Applications/CodeBuddy.app/Contents/Resources/app.asar",
		)
	default: // linux & others
		if home, err := os.UserHomeDir(); err == nil && home != "" {
			paths = append(paths,
				filepath.Join(home, ".local", "share", "WorkBuddy", "resources", "app.asar"),
				filepath.Join(home, ".local", "share", "workbuddy", "resources", "app.asar"),
				filepath.Join(home, ".local", "share", "CodeBuddy", "resources", "app.asar"),
			)
		}
		paths = append(paths,
			"/opt/WorkBuddy/resources/app.asar",
			"/opt/CodeBuddy/resources/app.asar",
		)
	}
	return paths
}

// extractCodebuddyModels reads the official client bundle and returns the
// de-duplicated, sorted model list as ModelInfo entries.
func extractCodebuddyModels(asarPath string) ([]*ModelInfo, error) {
	f, err := os.Open(asarPath)
	if err != nil {
		return nil, err
	}
	defer func() { _ = f.Close() }()

	data, err := io.ReadAll(f)
	if err != nil {
		return nil, err
	}

	seen := make(map[string]*ModelInfo)
	for _, m := range codebuddyAsarModelRe.FindAllSubmatch(data, -1) {
		providerID := strings.TrimSpace(string(m[1]))
		modelID := strings.TrimSpace(string(m[2]))
		displayName := strings.TrimSpace(string(m[3]))
		supportsImages := strings.EqualFold(strings.TrimSpace(string(m[5])), "true")

		// 剥离 thinking 变体后缀（如 kimi-k3-1 → kimi-k3），对齐后端真实模型 ID。
		modelID = stripCodebuddyThinkingSuffix(modelID)

		if !isValidModelID(modelID) {
			continue
		}
		if _, exists := seen[modelID]; exists {
			// A model may appear under multiple provider groups with different
			// capability flags; merge conservatively (any group declaring image
			// support marks the model as vision-capable).
			seen[modelID].SupportsImages = seen[modelID].SupportsImages || supportsImages
			// Prefer a concrete display name over a generic "Auto" alias when the
			// same model id appears under multiple provider groups.
			if strings.EqualFold(displayName, "auto") {
				continue
			}
			if strings.EqualFold(seen[modelID].DisplayName, "auto") {
				seen[modelID].DisplayName = displayName
				seen[modelID].Description = codebuddyModelDescription(providerID)
			}
			continue
		}

		seen[modelID] = &ModelInfo{
			ID:                 modelID,
			Object:             "model",
			Created:            codebuddyModelCreatedAt,
			OwnedBy:            "tencent",
			Type:               "codebuddy",
			DisplayName:        displayName,
			Description:        codebuddyModelDescription(providerID),
			ContextLength:      200000,
			MaxCompletionTokens: 32768,
			SupportsImages:     supportsImages,
		}
	}

	if len(seen) == 0 {
		return nil, fmt.Errorf("no model definitions found in %s", asarPath)
	}

	result := make([]*ModelInfo, 0, len(seen))
	for _, m := range seen {
		result = append(result, m)
	}
	sort.Slice(result, func(i, j int) bool {
		return result[i].ID < result[j].ID
	})
	return result, nil
}

// SyncCodebuddyModelsFromOfficialClient extracts the official CodeBuddy model
// catalog from the locally installed WorkBuddy/CodeBuddy client bundle and
// installs it as the active registry catalog. On success it also returns the
// de-duplicated, sorted model ID list so callers can override the static
// manifest model IDs. It returns nil when the client is not installed or the
// bundle cannot be parsed.
//
// This is the authoritative local source of the real CodeBuddy model list,
// because the official Tencent backend exposes no public model-list API.
func SyncCodebuddyModelsFromOfficialClient() []string {
	asarPath := findCodebuddyAsarPath()
	if asarPath == "" {
		return nil
	}
	models, err := extractCodebuddyModels(asarPath)
	if err != nil || len(models) == 0 {
		return nil
	}

	codebuddySyncMu.Lock()
	codebuddySynced = models
	codebuddySyncMu.Unlock()

	ids := make([]string, 0, len(models))
	for _, m := range models {
		if m != nil && m.ID != "" {
			ids = append(ids, m.ID)
		}
	}
	return ids
}

// NotifyCodebuddyModelRefresh notifies the SDK service that the codebuddy model
// catalog changed, triggering re-registration of codebuddy auth models. Callers
// should invoke this after installing a new catalog via
// SyncCodebuddyModelsFromOfficialClient.
func NotifyCodebuddyModelRefresh() {
	notifyModelRefresh([]string{"codebuddy"})
}

// InstallCodebuddyModelIDs installs an externally-fetched model ID list (e.g.
// from the official backend model-list endpoint) as the active codebuddy
// registry catalog. It builds ModelInfo entries with default capability
// fields; prefer InstallCodebuddyModels when the backend response carries
// capability metadata (supportsImages, max tokens). It returns the installed,
// de-duplicated, sorted IDs, or nil when the input is empty.
func InstallCodebuddyModelIDs(ids []string) []string {
	if len(ids) == 0 {
		return nil
	}
	models := make([]*ModelInfo, 0, len(ids))
	for _, id := range ids {
		models = append(models, &ModelInfo{
			ID:          id,
			DisplayName: id,
			Description: "Tencent CodeBuddy model (synced from official backend).",
		})
	}
	return InstallCodebuddyModels(models)
}

// InstallCodebuddyModels installs an externally-fetched model list (e.g. from
// the official backend model-list endpoint) as the active codebuddy registry
// catalog, preserving capability fields such as SupportsImages so vision-proxy
// routing can correctly classify text-only vs vision-capable models. It
// returns the installed, de-duplicated, sorted IDs, or nil when the input is
// empty.
func InstallCodebuddyModels(models []*ModelInfo) []string {
	if len(models) == 0 {
		return nil
	}

	seen := make(map[string]*ModelInfo)
	for _, m := range models {
		if m == nil {
			continue
		}
		id := stripCodebuddyThinkingSuffix(strings.TrimSpace(m.ID))
		if id == "" || !isValidModelID(id) {
			continue
		}
		if existing, exists := seen[id]; exists {
			// 同一模型可能出现在多个 provider 组，能力字段保守合并。
			if m.SupportsImages {
				existing.SupportsImages = true
			}
			continue
		}
		m.ID = id
		if m.Object == "" {
			m.Object = "model"
		}
		if m.Created == 0 {
			m.Created = codebuddyModelCreatedAt
		}
		if m.OwnedBy == "" {
			m.OwnedBy = "tencent"
		}
		if m.Type == "" {
			m.Type = "codebuddy"
		}
		if m.DisplayName == "" {
			m.DisplayName = id
		}
		if m.ContextLength <= 0 {
			m.ContextLength = 200000
		}
		if m.MaxCompletionTokens <= 0 {
			m.MaxCompletionTokens = 32768
		}
		seen[id] = m
	}
	if len(seen) == 0 {
		return nil
	}

	out := make([]*ModelInfo, 0, len(seen))
	for _, m := range seen {
		out = append(out, m)
	}
	sort.Slice(out, func(i, j int) bool {
		return out[i].ID < out[j].ID
	})

	codebuddySyncMu.Lock()
	changed := modelSectionChanged(codebuddySynced, out)
	codebuddySynced = out
	codebuddySyncMu.Unlock()

	if changed {
		log.Infof("codebuddy model sync: backend list updated (%d models)", len(out))
		notifyModelRefresh([]string{"codebuddy"})
	} else {
		log.Infof("codebuddy model sync: backend list unchanged (%d models)", len(out))
	}

	result := make([]string, 0, len(out))
	for _, m := range out {
		if m != nil && m.ID != "" {
			result = append(result, m.ID)
		}
	}
	return result
}

func codebuddyModelDescription(providerID string) string {
	if providerID == "" {
		return "Tencent CodeBuddy model."
	}
	return fmt.Sprintf("Tencent CodeBuddy model (provider: %s).", providerID)
}

// codebuddyThinkingSuffixes are the model-ID suffixes the official CodeBuddy
// client appends to mark thinking variants. For example the app.asar advertises
// `kimi-k3-1` (kimi-k3 in auto-thinking mode), but the Tencent backend routes
// by the base model ID `kimi-k3` only. The `-1` marker is a client-side UI
// concern and must be stripped before the ID is sent upstream or exposed via
// /v1/models — otherwise routing fails with auth_not_found (the backend has no
// auth bound to `kimi-k3-1`).
var codebuddyThinkingSuffixes = []string{"-1"}

// stripCodebuddyThinkingSuffix removes a trailing thinking-variant suffix from a
// CodeBuddy model ID (e.g. `kimi-k3-1` → `kimi-k3`). IDs without a known suffix
// are returned unchanged.
func stripCodebuddyThinkingSuffix(id string) string {
	for _, suffix := range codebuddyThinkingSuffixes {
		if strings.HasSuffix(id, suffix) {
			if base := strings.TrimSuffix(id, suffix); base != "" {
				return base
			}
		}
	}
	return id
}

// isValidModelID reports whether id looks like a concrete model identifier
// rather than a false positive from the bundle scan.
func isValidModelID(id string) bool {
	if id == "" || len(id) > 128 {
		return false
	}
	for _, r := range id {
		switch {
		case r >= 'a' && r <= 'z':
		case r >= 'A' && r <= 'Z':
		case r >= '0' && r <= '9':
		case r == '-' || r == '_' || r == '.' || r == ':':
		default:
			return false
		}
	}
	return true
}

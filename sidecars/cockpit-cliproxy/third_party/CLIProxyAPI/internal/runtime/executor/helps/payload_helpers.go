package helps

import (
	"encoding/json"
	"net/http"
	"reflect"
	"strconv"
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/thinking"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

const (
	DisableImageGenerationHeader = "X-Agtools-Disable-Image-Generation"
	CodexResponsesLiteHeader     = "X-OpenAI-Internal-Codex-Responses-Lite"
)

// IsCodexResponsesLiteRequest reports whether a feature-marker header is
// present or any model is marked Responses Lite in the Codex catalog. Header
// presence is authoritative even when its value is empty.
func IsCodexResponsesLiteRequest(headers http.Header, modelIDs ...string) bool {
	for name := range headers {
		if strings.EqualFold(strings.TrimSpace(name), CodexResponsesLiteHeader) {
			return true
		}
	}
	for _, modelID := range modelIDs {
		if registry.CodexClientModelUsesResponsesLite(modelID) {
			return true
		}
	}
	return false
}

func EffectiveDisableImageGenerationMode(cfg *config.Config, headers http.Header) config.DisableImageGenerationMode {
	mode := config.DisableImageGenerationOff
	if cfg != nil {
		mode = cfg.DisableImageGeneration
	}
	if mode == config.DisableImageGenerationAll {
		return mode
	}
	headerMode := disableImageGenerationModeFromHeader(headers)
	if headerMode == config.DisableImageGenerationAll {
		return headerMode
	}
	if mode == config.DisableImageGenerationChat || headerMode == config.DisableImageGenerationChat {
		return config.DisableImageGenerationChat
	}
	return mode
}

func ShouldInjectImageGenerationTool(cfg *config.Config, requestPath string, headers http.Header) bool {
	return ShouldInjectImageGenerationToolForModel(cfg, "", requestPath, headers)
}

func ShouldInjectImageGenerationToolForModel(cfg *config.Config, model, requestPath string, headers http.Header) bool {
	if IsCodexResponsesLiteRequest(headers, model) {
		return false
	}
	mode := EffectiveDisableImageGenerationMode(cfg, headers)
	return mode == config.DisableImageGenerationOff ||
		(mode == config.DisableImageGenerationChat && isImagesEndpointRequestPath(requestPath))
}

func disableImageGenerationModeFromHeader(headers http.Header) config.DisableImageGenerationMode {
	if headers == nil {
		return config.DisableImageGenerationOff
	}
	switch strings.TrimSpace(strings.ToLower(headers.Get(DisableImageGenerationHeader))) {
	case "true", "1", "on", "yes", "all", "disabled":
		return config.DisableImageGenerationAll
	case "chat", "images_only", "images-only":
		return config.DisableImageGenerationChat
	default:
		return config.DisableImageGenerationOff
	}
}

func shouldFilterImageGenerationPayload(mode config.DisableImageGenerationMode, requestPath string) bool {
	return mode != config.DisableImageGenerationOff &&
		(mode != config.DisableImageGenerationChat || !isImagesEndpointRequestPath(requestPath))
}

// ApplyPayloadConfigWithRoot behaves like applyPayloadConfig but treats all parameter
// paths as relative to the provided root path (for example, "request" for Gemini CLI)
// and restricts matches to the given protocol when supplied. Defaults are checked
// against the original payload when provided. requestedModel carries the client-visible
// model name before alias resolution so payload rules can target aliases precisely.
// requestPath is the inbound HTTP request path (when available) used for endpoint-scoped gates.
func ApplyPayloadConfigWithRoot(cfg *config.Config, model, protocol, root string, payload, original []byte, requestedModel string, requestPath string) []byte {
	return ApplyPayloadConfigWithRequest(cfg, model, protocol, "", root, payload, original, requestedModel, requestPath, nil)
}

// ApplyPayloadConfigWithRequest applies payload config using source protocol and request header gates.
func ApplyPayloadConfigWithRequest(cfg *config.Config, model, protocol, fromProtocol, root string, payload, original []byte, requestedModel string, requestPath string, headers http.Header) []byte {
	if len(payload) == 0 {
		return payload
	}
	out := payload

	// Apply config disable-image-generation filtering before payload rules so
	// conditions/defaults see the filtered shape. A final pass below enforces the
	// effective mode again after overrides.
	disableImageGeneration := config.DisableImageGenerationOff
	if cfg != nil {
		disableImageGeneration = cfg.DisableImageGeneration
	}
	if shouldFilterImageGenerationPayload(disableImageGeneration, requestPath) {
		out = removeImageGenerationToolsFromPayloadWithRoot(out, root)
	}

	if cfg == nil {
		return applyFinalPayloadGuards(out, cfg, root, model, requestedModel, requestPath, headers)
	}

	rules := cfg.Payload
	hasPayloadRules := len(rules.Default) != 0 || len(rules.DefaultRaw) != 0 || len(rules.Override) != 0 || len(rules.OverrideRaw) != 0 || len(rules.Filter) != 0
	if hasPayloadRules {
		model = strings.TrimSpace(model)
		requestedModel = strings.TrimSpace(requestedModel)
		if model != "" || requestedModel != "" {
			candidates := payloadModelCandidates(model, requestedModel)
			source := original
			if len(source) == 0 {
				source = payload
			}
			appliedDefaults := make(map[string]struct{})
			// Apply default rules: first write wins per field across all matching rules.
			for i := range rules.Default {
				rule := &rules.Default[i]
				if !payloadModelRulesMatch(rule.Models, protocol, fromProtocol, headers, out, root, candidates) {
					continue
				}
				for path, value := range rule.Params {
					fullPath := buildPayloadPath(root, path)
					if fullPath == "" {
						continue
					}
					for _, resolvedPath := range resolvePayloadRulePaths(out, fullPath) {
						if gjson.GetBytes(source, resolvedPath).Exists() {
							continue
						}
						if _, ok := appliedDefaults[resolvedPath]; ok {
							continue
						}
						updated, errSet := sjson.SetBytes(out, resolvedPath, value)
						if errSet != nil {
							continue
						}
						out = updated
						appliedDefaults[resolvedPath] = struct{}{}
					}
				}
			}
			// Apply default raw rules: first write wins per field across all matching rules.
			for i := range rules.DefaultRaw {
				rule := &rules.DefaultRaw[i]
				if !payloadModelRulesMatch(rule.Models, protocol, fromProtocol, headers, out, root, candidates) {
					continue
				}
				for path, value := range rule.Params {
					fullPath := buildPayloadPath(root, path)
					if fullPath == "" {
						continue
					}
					for _, resolvedPath := range resolvePayloadRulePaths(out, fullPath) {
						if gjson.GetBytes(source, resolvedPath).Exists() {
							continue
						}
						if _, ok := appliedDefaults[resolvedPath]; ok {
							continue
						}
						rawValue, ok := payloadRawValue(value)
						if !ok {
							continue
						}
						updated, errSet := sjson.SetRawBytes(out, resolvedPath, rawValue)
						if errSet != nil {
							continue
						}
						out = updated
						appliedDefaults[resolvedPath] = struct{}{}
					}
				}
			}
			// Apply override rules: last write wins per field across all matching rules.
			for i := range rules.Override {
				rule := &rules.Override[i]
				if !payloadModelRulesMatch(rule.Models, protocol, fromProtocol, headers, out, root, candidates) {
					continue
				}
				for path, value := range rule.Params {
					fullPath := buildPayloadPath(root, path)
					if fullPath == "" {
						continue
					}
					for _, resolvedPath := range resolvePayloadRulePaths(out, fullPath) {
						updated, errSet := sjson.SetBytes(out, resolvedPath, value)
						if errSet != nil {
							continue
						}
						out = updated
					}
				}
			}
			// Apply override raw rules: last write wins per field across all matching rules.
			for i := range rules.OverrideRaw {
				rule := &rules.OverrideRaw[i]
				if !payloadModelRulesMatch(rule.Models, protocol, fromProtocol, headers, out, root, candidates) {
					continue
				}
				for path, value := range rule.Params {
					fullPath := buildPayloadPath(root, path)
					if fullPath == "" {
						continue
					}
					rawValue, ok := payloadRawValue(value)
					if !ok {
						continue
					}
					for _, resolvedPath := range resolvePayloadRulePaths(out, fullPath) {
						updated, errSet := sjson.SetRawBytes(out, resolvedPath, rawValue)
						if errSet != nil {
							continue
						}
						out = updated
					}
				}
			}
			// Apply filter rules: remove matching paths from payload.
			for i := range rules.Filter {
				rule := &rules.Filter[i]
				if !payloadModelRulesMatch(rule.Models, protocol, fromProtocol, headers, out, root, candidates) {
					continue
				}
				for _, path := range rule.Params {
					fullPath := buildPayloadPath(root, path)
					if fullPath == "" {
						continue
					}
					resolvedPaths := resolvePayloadRulePaths(out, fullPath)
					for i := len(resolvedPaths) - 1; i >= 0; i-- {
						resolvedPath := resolvedPaths[i]
						updated, errDel := sjson.DeleteBytes(out, resolvedPath)
						if errDel != nil {
							continue
						}
						out = updated
					}
				}
			}
		}
	}
	return applyFinalPayloadGuards(out, cfg, root, model, requestedModel, requestPath, headers)
}

func applyFinalPayloadGuards(payload []byte, cfg *config.Config, root, model, requestedModel, requestPath string, headers http.Header) []byte {
	out := payload
	// These request-level gates must win over payload overrides that may have
	// restored unsupported tools.
	effectiveImageGeneration := EffectiveDisableImageGenerationMode(cfg, headers)
	if shouldFilterImageGenerationPayload(effectiveImageGeneration, requestPath) {
		out = removeImageGenerationToolsFromPayloadWithRoot(out, root)
	}
	// Responses Lite only accepts a small tool allowlist.
	//
	// Catalog Lite models always get the filter (including stripping any hosted
	// image_generation that payload rules may have reintroduced).
	//
	// Non-catalog requests that only carry the Lite feature header can still be
	// upgraded to full Responses for image generation (OAuth image_gen namespace
	// / API-key hosted image tools). Skip the allowlist filter in that case so
	// those tools survive until the executor decides. Hosted injection for Lite
	// is blocked separately by ShouldInjectImageGenerationToolForModel.
	if IsCodexResponsesLiteRequest(headers, model, requestedModel) {
		catalogLite := registry.CodexClientModelUsesResponsesLite(model) ||
			registry.CodexClientModelUsesResponsesLite(requestedModel)
		if catalogLite || !payloadDeclaresImageGenerationToolsWithRoot(out, root) {
			out = filterResponsesLiteToolsFromPayloadWithRoot(out, root)
		}
	}
	return out
}

// payloadDeclaresImageGenerationToolsWithRoot reports whether the payload
// already includes hosted image_generation tools, image_gen namespaces, or
// image_gen.imagegen function tools that should not be stripped before the
// executor decides whether to stay on Responses Lite.
func payloadDeclaresImageGenerationToolsWithRoot(payload []byte, root string) bool {
	if len(payload) == 0 {
		return false
	}
	objectPath := strings.TrimSpace(root)
	if payloadDeclaresImageGenerationToolsInObject(payload, objectPath) {
		return true
	}
	// Nested request wrappers used by some translators.
	requestPath := appendPayloadPathPart(objectPath, "request")
	if gjson.GetBytes(payload, requestPath).IsObject() &&
		payloadDeclaresImageGenerationToolsInObject(payload, requestPath) {
		return true
	}
	return false
}

func payloadDeclaresImageGenerationToolsInObject(payload []byte, objectPath string) bool {
	if toolArrayDeclaresImageGeneration(gjson.GetBytes(payload, appendPayloadPathPart(objectPath, "tools"))) {
		return true
	}
	if toolValueDeclaresImageGeneration(gjson.GetBytes(payload, appendPayloadPathPart(objectPath, "tool_choice"))) {
		return true
	}

	inputPath := appendPayloadPathPart(objectPath, "input")
	input := gjson.GetBytes(payload, inputPath)
	if input.IsArray() {
		for index, item := range input.Array() {
			if !strings.EqualFold(strings.TrimSpace(item.Get("type").String()), "additional_tools") {
				continue
			}
			itemPath := appendPayloadPathPart(inputPath, strconv.Itoa(index))
			if payloadDeclaresImageGenerationToolsInObject(payload, itemPath) {
				return true
			}
		}
	}

	responsePath := appendPayloadPathPart(objectPath, "response")
	if gjson.GetBytes(payload, responsePath).IsObject() &&
		payloadDeclaresImageGenerationToolsInObject(payload, responsePath) {
		return true
	}
	return false
}

func toolArrayDeclaresImageGeneration(tools gjson.Result) bool {
	if !tools.IsArray() {
		return false
	}
	for _, tool := range tools.Array() {
		if toolValueDeclaresImageGeneration(tool) {
			return true
		}
	}
	return false
}

func toolValueDeclaresImageGeneration(tool gjson.Result) bool {
	if !tool.Exists() {
		return false
	}
	if tool.Type == gjson.String {
		name := strings.ToLower(strings.TrimSpace(tool.String()))
		return name == "image_generation" || name == "image_gen.imagegen"
	}
	if !tool.IsObject() {
		return false
	}

	toolType := strings.ToLower(strings.TrimSpace(tool.Get("type").String()))
	name := strings.ToLower(strings.TrimSpace(tool.Get("name").String()))
	functionName := strings.ToLower(strings.TrimSpace(tool.Get("function.name").String()))

	switch toolType {
	case "image_generation", "image_gen", "image_gen.imagegen":
		return true
	case "namespace":
		if name == "image_gen" {
			return true
		}
	case "function", "custom", "tool":
		if name == "image_generation" || name == "image_gen.imagegen" || name == "imagegen" ||
			functionName == "image_generation" || functionName == "image_gen.imagegen" || functionName == "imagegen" {
			return true
		}
	}
	if name == "image_generation" || name == "image_gen.imagegen" || name == "image_gen" {
		return true
	}
	if nested := tool.Get("tools"); nested.IsArray() && toolArrayDeclaresImageGeneration(nested) {
		return true
	}
	return false
}

func isImagesEndpointRequestPath(path string) bool {
	path = strings.TrimSpace(path)
	if path == "" {
		return false
	}
	if path == "/v1/images/generations" || path == "/v1/images/edits" {
		return true
	}
	// Be tolerant of prefix routers that may report a longer matched route.
	if strings.HasSuffix(path, "/v1/images/generations") || strings.HasSuffix(path, "/v1/images/edits") {
		return true
	}
	if strings.HasSuffix(path, "/images/generations") || strings.HasSuffix(path, "/images/edits") {
		return true
	}
	return false
}

func payloadModelRulesMatch(rules []config.PayloadModelRule, protocol string, fromProtocol string, headers http.Header, payload []byte, root string, models []string) bool {
	if len(rules) == 0 || len(models) == 0 {
		return false
	}
	for _, model := range models {
		for _, entry := range rules {
			name := strings.TrimSpace(entry.Name)
			if name == "" {
				continue
			}
			if ep := strings.TrimSpace(entry.Protocol); ep != "" && protocol != "" && !strings.EqualFold(ep, protocol) {
				continue
			}
			if !payloadFromProtocolMatches(entry.FromProtocol, fromProtocol) {
				continue
			}
			if !payloadHeadersMatch(headers, entry.Headers) {
				continue
			}
			if !matchModelPattern(name, model) {
				continue
			}
			if payloadModelRuleConditionsMatch(payload, root, entry) {
				return true
			}
		}
	}
	return false
}

func payloadModelRuleConditionsMatch(payload []byte, root string, rule config.PayloadModelRule) bool {
	if !payloadMatchConditionsMatch(payload, root, rule.Match) {
		return false
	}
	if !payloadNotMatchConditionsMatch(payload, root, rule.NotMatch) {
		return false
	}
	if !payloadExistConditionsMatch(payload, root, rule.Exist) {
		return false
	}
	if !payloadNotExistConditionsMatch(payload, root, rule.NotExist) {
		return false
	}
	return true
}

func payloadMatchConditionsMatch(payload []byte, root string, conditions []map[string]any) bool {
	for _, condition := range conditions {
		for path, value := range condition {
			if strings.TrimSpace(path) == "" {
				continue
			}
			if !payloadPathMatchesValue(payload, buildPayloadPath(root, path), value) {
				return false
			}
		}
	}
	return true
}

func payloadNotMatchConditionsMatch(payload []byte, root string, conditions []map[string]any) bool {
	for _, condition := range conditions {
		for path, value := range condition {
			if strings.TrimSpace(path) == "" {
				continue
			}
			if payloadPathMatchesValue(payload, buildPayloadPath(root, path), value) {
				return false
			}
		}
	}
	return true
}

func payloadExistConditionsMatch(payload []byte, root string, paths []string) bool {
	for _, path := range paths {
		if strings.TrimSpace(path) == "" {
			continue
		}
		if !payloadPathExists(payload, buildPayloadPath(root, path)) {
			return false
		}
	}
	return true
}

func payloadNotExistConditionsMatch(payload []byte, root string, paths []string) bool {
	for _, path := range paths {
		if strings.TrimSpace(path) == "" {
			continue
		}
		if payloadPathExists(payload, buildPayloadPath(root, path)) {
			return false
		}
	}
	return true
}

func payloadPathMatchesValue(payload []byte, path string, value any) bool {
	for _, resolvedPath := range resolvePayloadRulePaths(payload, path) {
		result := gjson.GetBytes(payload, resolvedPath)
		if !result.Exists() {
			continue
		}
		if payloadResultEquals(result, value) {
			return true
		}
	}
	return false
}

func payloadPathExists(payload []byte, path string) bool {
	for _, resolvedPath := range resolvePayloadRulePaths(payload, path) {
		result := gjson.GetBytes(payload, resolvedPath)
		if result.Exists() && result.Type != gjson.Null {
			return true
		}
	}
	return false
}

func payloadResultEquals(result gjson.Result, value any) bool {
	actual, ok := normalizedPayloadResult(result)
	if !ok {
		return false
	}
	expected, ok := normalizedPayloadValue(value)
	if !ok {
		return false
	}
	return reflect.DeepEqual(actual, expected)
}

func normalizedPayloadResult(result gjson.Result) (any, bool) {
	if !result.Exists() {
		return nil, false
	}
	raw := strings.TrimSpace(result.Raw)
	if raw == "" {
		encoded, errMarshal := json.Marshal(result.Value())
		if errMarshal != nil {
			return nil, false
		}
		raw = string(encoded)
	}
	return normalizedPayloadJSON([]byte(raw))
}

func normalizedPayloadValue(value any) (any, bool) {
	encoded, errMarshal := json.Marshal(value)
	if errMarshal != nil {
		return nil, false
	}
	return normalizedPayloadJSON(encoded)
}

func normalizedPayloadJSON(data []byte) (any, bool) {
	if len(strings.TrimSpace(string(data))) == 0 {
		return nil, false
	}
	var out any
	if errUnmarshal := json.Unmarshal(data, &out); errUnmarshal != nil {
		return nil, false
	}
	return out, true
}

func payloadFromProtocolMatches(pattern, fromProtocol string) bool {
	pattern = normalizePayloadFromProtocol(pattern)
	if pattern == "" {
		return true
	}
	fromProtocol = normalizePayloadFromProtocol(fromProtocol)
	if fromProtocol == "" {
		return false
	}
	return strings.EqualFold(pattern, fromProtocol)
}

func normalizePayloadFromProtocol(protocol string) string {
	protocol = strings.ToLower(strings.TrimSpace(protocol))
	switch protocol {
	case "openai-response", "openai-responses", "response":
		return "responses"
	case "gemini-cli":
		return "gemini"
	default:
		return protocol
	}
}

func payloadHeadersMatch(headers http.Header, rules map[string]string) bool {
	if len(rules) == 0 {
		return true
	}
	for key, pattern := range rules {
		key = strings.TrimSpace(key)
		if key == "" {
			continue
		}
		values := payloadHeaderValues(headers, key)
		if len(values) == 0 {
			return false
		}
		matched := false
		for _, value := range values {
			if matchModelPattern(pattern, value) {
				matched = true
				break
			}
		}
		if !matched {
			return false
		}
	}
	return true
}

func payloadHeaderValues(headers http.Header, key string) []string {
	if headers == nil {
		return nil
	}
	var values []string
	for headerKey, headerValues := range headers {
		if strings.EqualFold(headerKey, key) {
			values = append(values, headerValues...)
		}
	}
	return values
}

func payloadModelCandidates(model, requestedModel string) []string {
	model = strings.TrimSpace(model)
	requestedModel = strings.TrimSpace(requestedModel)
	if model == "" && requestedModel == "" {
		return nil
	}
	candidates := make([]string, 0, 3)
	seen := make(map[string]struct{}, 3)
	addCandidate := func(value string) {
		value = strings.TrimSpace(value)
		if value == "" {
			return
		}
		key := strings.ToLower(value)
		if _, ok := seen[key]; ok {
			return
		}
		seen[key] = struct{}{}
		candidates = append(candidates, value)
	}
	if model != "" {
		addCandidate(model)
	}
	if requestedModel != "" {
		parsed := thinking.ParseSuffix(requestedModel)
		base := strings.TrimSpace(parsed.ModelName)
		if base != "" {
			addCandidate(base)
		}
		if parsed.HasSuffix {
			addCandidate(requestedModel)
		}
	}
	return candidates
}

// buildPayloadPath combines an optional root path with a relative parameter path.
// When root is empty, the parameter path is used as-is. When root is non-empty,
// the parameter path is treated as relative to root.
func buildPayloadPath(root, path string) string {
	r := strings.TrimSpace(root)
	p := strings.TrimSpace(path)
	if r == "" {
		return p
	}
	if p == "" {
		return r
	}
	if strings.HasPrefix(p, ".") {
		p = p[1:]
	}
	return r + "." + p
}

func resolvePayloadRulePaths(payload []byte, path string) []string {
	path = strings.TrimSpace(path)
	if path == "" {
		return nil
	}
	if !strings.Contains(path, "#(") {
		return []string{path}
	}
	parts := splitPayloadRulePath(path)
	if len(parts) == 0 {
		return nil
	}
	paths := []string{""}
	for _, part := range parts {
		query, allMatches, ok := parsePayloadQueryPathPart(part)
		if !ok {
			for i := range paths {
				paths[i] = appendPayloadPathPart(paths[i], part)
			}
			continue
		}
		nextPaths := make([]string, 0, len(paths))
		for _, basePath := range paths {
			array := payloadValueAtPath(payload, basePath)
			if !array.Exists() || !array.IsArray() {
				continue
			}
			for index, item := range array.Array() {
				if !payloadQueryMatches(item, query) {
					continue
				}
				nextPaths = append(nextPaths, appendPayloadPathPart(basePath, strconv.Itoa(index)))
				if !allMatches {
					break
				}
			}
		}
		paths = nextPaths
		if len(paths) == 0 {
			return nil
		}
	}
	return paths
}

func splitPayloadRulePath(path string) []string {
	var parts []string
	start := 0
	depth := 0
	var quote byte
	escaped := false
	for i := 0; i < len(path); i++ {
		ch := path[i]
		if escaped {
			escaped = false
			continue
		}
		if ch == '\\' {
			escaped = true
			continue
		}
		if quote != 0 {
			if ch == quote {
				quote = 0
			}
			continue
		}
		if ch == '"' || ch == '\'' {
			quote = ch
			continue
		}
		if ch == '(' {
			depth++
			continue
		}
		if ch == ')' {
			if depth > 0 {
				depth--
			}
			continue
		}
		if ch == '.' && depth == 0 {
			parts = append(parts, path[start:i])
			start = i + 1
		}
	}
	parts = append(parts, path[start:])
	return parts
}

func parsePayloadQueryPathPart(part string) (string, bool, bool) {
	if !strings.HasPrefix(part, "#(") {
		return "", false, false
	}
	closeIndex := findPayloadQueryClose(part)
	if closeIndex < 0 {
		return "", false, false
	}
	suffix := part[closeIndex+1:]
	if suffix != "" && suffix != "#" {
		return "", false, false
	}
	return strings.TrimSpace(part[2:closeIndex]), suffix == "#", true
}

func findPayloadQueryClose(part string) int {
	var quote byte
	escaped := false
	depth := 1
	for i := 2; i < len(part); i++ {
		ch := part[i]
		if escaped {
			escaped = false
			continue
		}
		if ch == '\\' {
			escaped = true
			continue
		}
		if quote != 0 {
			if ch == quote {
				quote = 0
			}
			continue
		}
		if ch == '"' || ch == '\'' {
			quote = ch
			continue
		}
		if ch == '(' {
			depth++
			continue
		}
		if ch == ')' {
			depth--
			if depth == 0 {
				return i
			}
		}
	}
	return -1
}

func appendPayloadPathPart(path, part string) string {
	if path == "" {
		return part
	}
	if part == "" {
		return path
	}
	return path + "." + part
}

func payloadValueAtPath(payload []byte, path string) gjson.Result {
	if path == "" {
		return gjson.ParseBytes(payload)
	}
	return gjson.GetBytes(payload, path)
}

func payloadQueryMatches(item gjson.Result, query string) bool {
	for _, orPart := range splitPayloadLogical(query, "||") {
		if payloadQueryAndMatches(item, orPart) {
			return true
		}
	}
	return false
}

func payloadQueryAndMatches(item gjson.Result, query string) bool {
	parts := splitPayloadLogical(query, "&&")
	if len(parts) == 0 {
		return false
	}
	for _, part := range parts {
		if !payloadQueryTermMatches(item, part) {
			return false
		}
	}
	return true
}

func splitPayloadLogical(query, operator string) []string {
	var parts []string
	start := 0
	var quote byte
	escaped := false
	for i := 0; i < len(query); i++ {
		ch := query[i]
		if escaped {
			escaped = false
			continue
		}
		if ch == '\\' {
			escaped = true
			continue
		}
		if quote != 0 {
			if ch == quote {
				quote = 0
			}
			continue
		}
		if ch == '"' || ch == '\'' {
			quote = ch
			continue
		}
		if strings.HasPrefix(query[i:], operator) {
			parts = append(parts, strings.TrimSpace(query[start:i]))
			i += len(operator) - 1
			start = i + 1
		}
	}
	parts = append(parts, strings.TrimSpace(query[start:]))
	return parts
}

func payloadQueryTermMatches(item gjson.Result, term string) bool {
	term = strings.TrimSpace(term)
	if term == "" || item.Raw == "" {
		return false
	}
	wrapped := make([]byte, 0, len(item.Raw)+2)
	wrapped = append(wrapped, '[')
	wrapped = append(wrapped, item.Raw...)
	wrapped = append(wrapped, ']')
	return gjson.GetBytes(wrapped, "#("+term+")").Exists()
}

func removeToolTypeFromPayloadWithRoot(payload []byte, root string, toolType string) []byte {
	if len(payload) == 0 {
		return payload
	}
	toolType = strings.TrimSpace(toolType)
	if toolType == "" {
		return payload
	}
	toolsPath := buildPayloadPath(root, "tools")
	return removeToolTypeFromToolsArray(payload, toolsPath, toolType)
}

// filterResponsesLiteToolsFromPayloadWithRoot enforces the Responses Lite tool
// allowlist wherever Responses request metadata can declare tools. Historical
// response metadata is nested under response, while dynamically loaded tools
// are carried by input items of type additional_tools.
func filterResponsesLiteToolsFromPayloadWithRoot(payload []byte, root string) []byte {
	if len(payload) == 0 {
		return payload
	}
	return filterResponsesLiteToolsFromObject(payload, strings.TrimSpace(root))
}

func filterResponsesLiteToolsFromObject(payload []byte, objectPath string) []byte {
	out, _, _ := filterResponsesLiteToolsArray(payload, appendPayloadPathPart(objectPath, "tools"))
	out = filterResponsesLiteToolChoice(out, appendPayloadPathPart(objectPath, "tool_choice"))

	inputPath := appendPayloadPathPart(objectPath, "input")
	input := gjson.GetBytes(out, inputPath)
	if input.IsArray() {
		items := input.Array()
		for index := len(items) - 1; index >= 0; index-- {
			if !strings.EqualFold(strings.TrimSpace(items[index].Get("type").String()), "additional_tools") {
				continue
			}
			itemPath := appendPayloadPathPart(inputPath, strconv.Itoa(index))
			out = filterResponsesLiteToolsFromObject(out, itemPath)
			tools := gjson.GetBytes(out, appendPayloadPathPart(itemPath, "tools"))
			if !tools.IsArray() || len(tools.Array()) == 0 {
				if updated, errDel := sjson.DeleteBytes(out, itemPath); errDel == nil {
					out = updated
				}
			}
		}
	}

	responsePath := appendPayloadPathPart(objectPath, "response")
	if response := gjson.GetBytes(out, responsePath); response.IsObject() {
		out = filterResponsesLiteToolsFromObject(out, responsePath)
	}
	return out
}

func filterResponsesLiteToolsArray(payload []byte, toolsPath string) ([]byte, bool, bool) {
	tools := gjson.GetBytes(payload, toolsPath)
	if !tools.IsArray() {
		return payload, false, false
	}

	changed := false
	filtered := []byte(`[]`)
	for _, tool := range tools.Array() {
		if !responsesLiteToolAllowed(tool) {
			changed = true
			continue
		}
		updated, errSet := sjson.SetRawBytes(filtered, "-1", []byte(tool.Raw))
		if errSet != nil {
			return payload, false, len(tools.Array()) > 0
		}
		filtered = updated
	}
	if !changed {
		return payload, false, len(tools.Array()) > 0
	}
	updated, errSet := sjson.SetRawBytes(payload, toolsPath, filtered)
	if errSet != nil {
		return payload, false, len(tools.Array()) > 0
	}
	return updated, true, len(gjson.ParseBytes(filtered).Array()) > 0
}

func responsesLiteToolAllowed(tool gjson.Result) bool {
	if !tool.IsObject() {
		return false
	}
	switch strings.ToLower(strings.TrimSpace(tool.Get("type").String())) {
	case "function", "custom", "namespace":
		return true
	case "tool_search":
		return strings.EqualFold(strings.TrimSpace(tool.Get("execution").String()), "client")
	default:
		return false
	}
}

func filterResponsesLiteToolChoice(payload []byte, toolChoicePath string) []byte {
	choice := gjson.GetBytes(payload, toolChoicePath)
	if !choice.Exists() {
		return payload
	}
	if choice.Type == gjson.String {
		switch strings.ToLower(strings.TrimSpace(choice.String())) {
		case "auto", "none", "required":
			return payload
		default:
			return deletePayloadPath(payload, toolChoicePath)
		}
	}
	if !choice.IsObject() {
		return deletePayloadPath(payload, toolChoicePath)
	}

	choiceType := strings.ToLower(strings.TrimSpace(choice.Get("type").String()))
	switch choiceType {
	case "function", "custom", "namespace":
		return payload
	case "tool_search":
		if responsesLiteToolAllowed(choice) {
			return payload
		}
		return deletePayloadPath(payload, toolChoicePath)
	case "allowed_tools":
		updated := payload
		hasAllowedTools := false
		for _, relativePath := range []string{"tools", "allowed_tools", "allowed_tools.tools"} {
			path := appendPayloadPathPart(toolChoicePath, relativePath)
			var hasTools bool
			updated, _, hasTools = filterResponsesLiteToolsArray(updated, path)
			hasAllowedTools = hasAllowedTools || hasTools
		}
		if hasAllowedTools {
			return updated
		}
		return deletePayloadPath(updated, toolChoicePath)
	default:
		return deletePayloadPath(payload, toolChoicePath)
	}
}

func deletePayloadPath(payload []byte, path string) []byte {
	updated, errDel := sjson.DeleteBytes(payload, path)
	if errDel != nil {
		return payload
	}
	return updated
}

// removeImageGenerationToolsFromPayloadWithRoot removes all image-generation tool
// declarations understood by the Responses APIs. Besides the hosted
// image_generation tool, Responses Lite can expose image generation as an
// image_gen namespace or as the image_gen.imagegen function. additional_tools
// input items carry the same declarations in a nested tools array.
func removeImageGenerationToolsFromPayloadWithRoot(payload []byte, root string) []byte {
	if len(payload) == 0 {
		return payload
	}

	out := removeImageGenerationToolsArray(payload, buildPayloadPath(root, "tools"))
	out = removeImageGenerationToolChoice(out, buildPayloadPath(root, "tool_choice"))

	inputPath := buildPayloadPath(root, "input")
	input := gjson.GetBytes(out, inputPath)
	if !input.Exists() || !input.IsArray() {
		return out
	}
	inputItems := input.Array()
	for index := len(inputItems) - 1; index >= 0; index-- {
		item := inputItems[index]
		if !strings.EqualFold(strings.TrimSpace(item.Get("type").String()), "additional_tools") {
			continue
		}
		itemPath := appendPayloadPathPart(inputPath, strconv.Itoa(index))
		toolsPath := appendPayloadPathPart(itemPath, "tools")
		toolsBefore := gjson.GetBytes(out, toolsPath)
		out = removeImageGenerationToolsArray(out, toolsPath)
		out = removeImageGenerationToolChoice(out, appendPayloadPathPart(itemPath, "tool_choice"))
		if toolsBefore.IsArray() && len(toolsBefore.Array()) > 0 && len(gjson.GetBytes(out, toolsPath).Array()) == 0 {
			if updated, errDel := sjson.DeleteBytes(out, itemPath); errDel == nil {
				out = updated
			}
		}
	}
	return out
}

func removeImageGenerationToolsArray(payload []byte, toolsPath string) []byte {
	tools := gjson.GetBytes(payload, toolsPath)
	if !tools.Exists() || !tools.IsArray() {
		return payload
	}
	removed := false
	filtered := []byte(`[]`)
	for _, tool := range tools.Array() {
		if isImageGenerationToolReference(tool) {
			removed = true
			continue
		}
		updated, errSet := sjson.SetRawBytes(filtered, "-1", []byte(tool.Raw))
		if errSet != nil {
			return payload
		}
		filtered = updated
	}
	if !removed {
		return payload
	}
	updated, errSet := sjson.SetRawBytes(payload, toolsPath, filtered)
	if errSet != nil {
		return payload
	}
	return updated
}

func removeImageGenerationToolChoice(payload []byte, toolChoicePath string) []byte {
	choice := gjson.GetBytes(payload, toolChoicePath)
	if !choice.Exists() {
		return payload
	}
	if isImageGenerationToolReference(choice) {
		updated, errDel := sjson.DeleteBytes(payload, toolChoicePath)
		if errDel == nil {
			return updated
		}
		return payload
	}
	if choice.Type != gjson.JSON {
		return payload
	}

	choiceToolsPath := appendPayloadPathPart(toolChoicePath, "tools")
	choiceToolsBefore := gjson.GetBytes(payload, choiceToolsPath)
	updated := removeImageGenerationToolsArray(payload, choiceToolsPath)
	if choiceToolsBefore.IsArray() && len(choiceToolsBefore.Array()) > 0 && len(gjson.GetBytes(updated, choiceToolsPath).Array()) == 0 {
		if withoutChoice, errDel := sjson.DeleteBytes(updated, toolChoicePath); errDel == nil {
			return withoutChoice
		}
	}
	allowedToolsPath := appendPayloadPathPart(toolChoicePath, "allowed_tools")
	allowedTools := gjson.GetBytes(updated, allowedToolsPath)
	if allowedTools.IsArray() {
		updated = removeImageGenerationToolsArray(updated, allowedToolsPath)
		if len(allowedTools.Array()) > 0 && len(gjson.GetBytes(updated, allowedToolsPath).Array()) == 0 {
			if withoutChoice, errDel := sjson.DeleteBytes(updated, toolChoicePath); errDel == nil {
				return withoutChoice
			}
		}
	} else if allowedTools.Type == gjson.JSON {
		nestedToolsPath := appendPayloadPathPart(allowedToolsPath, "tools")
		nestedToolsBefore := gjson.GetBytes(updated, nestedToolsPath)
		updated = removeImageGenerationToolsArray(updated, nestedToolsPath)
		if nestedToolsBefore.IsArray() && len(nestedToolsBefore.Array()) > 0 && len(gjson.GetBytes(updated, nestedToolsPath).Array()) == 0 {
			if withoutChoice, errDel := sjson.DeleteBytes(updated, toolChoicePath); errDel == nil {
				return withoutChoice
			}
		}
	}
	return updated
}

func isImageGenerationToolReference(tool gjson.Result) bool {
	if tool.Type == gjson.String {
		return isImageGenerationToolName(tool.String())
	}
	if tool.Type != gjson.JSON {
		return false
	}
	for _, nestedPath := range []string{"tool", "function"} {
		nested := tool.Get(nestedPath)
		if nested.Exists() && (isImageGenerationToolReference(nested) || isImageGenerationToolName(nested.Get("name").String())) {
			return true
		}
	}

	toolType := strings.TrimSpace(tool.Get("type").String())
	switch strings.ToLower(toolType) {
	case "image_generation":
		return true
	case "namespace":
		return strings.EqualFold(strings.TrimSpace(tool.Get("name").String()), "image_gen") ||
			strings.EqualFold(strings.TrimSpace(tool.Get("namespace").String()), "image_gen")
	case "function":
		name := strings.TrimSpace(tool.Get("name").String())
		if name == "" {
			name = strings.TrimSpace(tool.Get("function.name").String())
		}
		return strings.EqualFold(name, "image_gen.imagegen")
	case "tool":
		return isImageGenerationToolName(tool.Get("name").String())
	default:
		return false
	}
}

func isImageGenerationToolName(name string) bool {
	switch strings.ToLower(strings.TrimSpace(name)) {
	case "image_generation", "image_gen", "image_gen.imagegen":
		return true
	default:
		return false
	}
}

func removeToolChoiceFromPayloadWithRoot(payload []byte, root string, toolType string) []byte {
	if len(payload) == 0 {
		return payload
	}
	toolType = strings.TrimSpace(toolType)
	if toolType == "" {
		return payload
	}
	toolChoicePath := buildPayloadPath(root, "tool_choice")
	return removeToolChoiceFromPayload(payload, toolChoicePath, toolType)
}

func removeToolChoiceFromPayload(payload []byte, toolChoicePath string, toolType string) []byte {
	choice := gjson.GetBytes(payload, toolChoicePath)
	if !choice.Exists() {
		return payload
	}
	if choice.Type == gjson.String {
		if strings.EqualFold(strings.TrimSpace(choice.String()), toolType) {
			updated, errDel := sjson.DeleteBytes(payload, toolChoicePath)
			if errDel == nil {
				return updated
			}
		}
		return payload
	}
	if choice.Type != gjson.JSON {
		return payload
	}
	choiceType := strings.TrimSpace(choice.Get("type").String())
	if strings.EqualFold(choiceType, toolType) {
		updated, errDel := sjson.DeleteBytes(payload, toolChoicePath)
		if errDel == nil {
			return updated
		}
		return payload
	}
	if strings.EqualFold(choiceType, "tool") {
		name := strings.TrimSpace(choice.Get("name").String())
		if strings.EqualFold(name, toolType) {
			updated, errDel := sjson.DeleteBytes(payload, toolChoicePath)
			if errDel == nil {
				return updated
			}
		}
	}
	return payload
}

func removeToolTypeFromToolsArray(payload []byte, toolsPath string, toolType string) []byte {
	tools := gjson.GetBytes(payload, toolsPath)
	if !tools.Exists() || !tools.IsArray() {
		return payload
	}
	removed := false
	filtered := []byte(`[]`)
	for _, tool := range tools.Array() {
		if tool.Get("type").String() == toolType {
			removed = true
			continue
		}
		updated, errSet := sjson.SetRawBytes(filtered, "-1", []byte(tool.Raw))
		if errSet != nil {
			continue
		}
		filtered = updated
	}
	if !removed {
		return payload
	}
	updated, errSet := sjson.SetRawBytes(payload, toolsPath, filtered)
	if errSet != nil {
		return payload
	}
	return updated
}

func payloadRawValue(value any) ([]byte, bool) {
	if value == nil {
		return nil, false
	}
	switch typed := value.(type) {
	case string:
		return []byte(typed), true
	case []byte:
		return typed, true
	default:
		raw, errMarshal := json.Marshal(typed)
		if errMarshal != nil {
			return nil, false
		}
		return raw, true
	}
}

func PayloadRequestedModel(opts cliproxyexecutor.Options, fallback string) string {
	fallback = strings.TrimSpace(fallback)
	if len(opts.Metadata) == 0 {
		return fallback
	}
	raw, ok := opts.Metadata[cliproxyexecutor.RequestedModelMetadataKey]
	if !ok || raw == nil {
		return fallback
	}
	switch v := raw.(type) {
	case string:
		if strings.TrimSpace(v) == "" {
			return fallback
		}
		return strings.TrimSpace(v)
	case []byte:
		if len(v) == 0 {
			return fallback
		}
		trimmed := strings.TrimSpace(string(v))
		if trimmed == "" {
			return fallback
		}
		return trimmed
	default:
		return fallback
	}
}

func PayloadRequestPath(opts cliproxyexecutor.Options) string {
	if len(opts.Metadata) == 0 {
		return ""
	}
	raw, ok := opts.Metadata[cliproxyexecutor.RequestPathMetadataKey]
	if !ok || raw == nil {
		return ""
	}
	switch v := raw.(type) {
	case string:
		return strings.TrimSpace(v)
	case []byte:
		return strings.TrimSpace(string(v))
	default:
		return ""
	}
}

// matchModelPattern performs simple wildcard matching where '*' matches zero or more characters.
// Examples:
//
//	"*-5" matches "gpt-5"
//	"gpt-*" matches "gpt-5" and "gpt-4"
//	"gemini-*-pro" matches "gemini-2.5-pro" and "gemini-3-pro".
func matchModelPattern(pattern, model string) bool {
	pattern = strings.TrimSpace(pattern)
	model = strings.TrimSpace(model)
	if pattern == "" {
		return false
	}
	if pattern == "*" {
		return true
	}
	// Iterative glob-style matcher supporting only '*' wildcard.
	pi, si := 0, 0
	starIdx := -1
	matchIdx := 0
	for si < len(model) {
		if pi < len(pattern) && (pattern[pi] == model[si]) {
			pi++
			si++
			continue
		}
		if pi < len(pattern) && pattern[pi] == '*' {
			starIdx = pi
			matchIdx = si
			pi++
			continue
		}
		if starIdx != -1 {
			pi = starIdx + 1
			matchIdx++
			si = matchIdx
			continue
		}
		return false
	}
	for pi < len(pattern) && pattern[pi] == '*' {
		pi++
	}
	return pi == len(pattern)
}

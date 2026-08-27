// Package config provides configuration management for the CLI Proxy API server.
// It handles loading and parsing YAML configuration files, and provides structured
// access to application settings including server port, authentication directory,
// debug settings, proxy configuration, and API keys.
package config

import "strings"

// SDKConfig represents the application's configuration, loaded from a YAML file.
type SDKConfig struct {
	EnableGeminiCLIEndpoint bool `yaml:"enable-gemini-cli-endpoint,omitempty" json:"enable-gemini-cli-endpoint,omitempty"`
	// ProxyURL is the URL of an optional proxy server to use for outbound requests.
	ProxyURL string `yaml:"proxy-url" json:"proxy-url"`

	// DisableImageGeneration controls whether the built-in image_generation tool is injected/allowed.
	//
	// Supported values:
	//   - false (default): image_generation is enabled everywhere (normal behavior).
	//   - true: image_generation is disabled everywhere. The server stops injecting it, removes it from request payloads,
	//     and returns 404 for /v1/images/generations and /v1/images/edits.
	//   - "chat": disable image_generation injection for all non-images endpoints (e.g. /v1/responses, /v1/chat/completions),
	//     while keeping /v1/images/generations and /v1/images/edits enabled and preserving image_generation there.
	//   - "passthrough": do not modify the tool list on non-images endpoints — keep image_generation if the client
	//     sent it and do not inject it otherwise; on /v1/images/generations and /v1/images/edits behave like "chat".
	DisableImageGeneration DisableImageGenerationMode `yaml:"disable-image-generation" json:"disable-image-generation"`

	// GPTImage2BaseModel sets the base (mainline) model used by the legacy hosted
	// image_generation tool path when a Codex image request is not proxied directly
	// through the Image API.
	//
	// The value must start with "gpt-" (case-insensitive). If empty or invalid, the
	// default base model ("gpt-5.4-mini") is used.
	GPTImage2BaseModel string `yaml:"gpt-image-2-base-model,omitempty" json:"gpt-image-2-base-model,omitempty"`

	// VideoResultAuthCacheTTL controls how long video IDs stay pinned to the credential
	// that created them. Accepts duration strings like "30m" or "3h".
	// Empty or invalid values use the default 3h.
	VideoResultAuthCacheTTL string `yaml:"video-result-auth-cache-ttl,omitempty" json:"video-result-auth-cache-ttl,omitempty"`

	// ForceModelPrefix requires explicit model prefixes (e.g., "teamA/gemini-3-pro-preview")
	// to target prefixed credentials. When false, unprefixed model requests may use prefixed
	// credentials as well.
	ForceModelPrefix bool `yaml:"force-model-prefix" json:"force-model-prefix"`

	// RequestLog enables or disables detailed request logging functionality.
	RequestLog bool `yaml:"request-log" json:"request-log"`

	// CodexOptimizeMultiAgentV2 mirrors the provider-wide runtime setting for API handlers.
	CodexOptimizeMultiAgentV2 bool `yaml:"-" json:"-"`

	// ClaudeCode configures Claude Code compatibility behavior.
	ClaudeCode ClaudeCodeConfig `yaml:"claude-code" json:"claude-code"`

	// APIKeys is a list of keys for authenticating clients to this proxy server.
	APIKeys []string `yaml:"api-keys" json:"api-keys"`

	// PassthroughHeaders controls whether upstream response headers are forwarded to downstream clients.
	// Default is false (disabled).
	PassthroughHeaders bool `yaml:"passthrough-headers" json:"passthrough-headers"`

	// Streaming configures server-side streaming behavior (keep-alives and safe bootstrap retries).
	Streaming StreamingConfig `yaml:"streaming" json:"streaming"`

	// NonStreamKeepAliveInterval controls how often blank lines are emitted for non-streaming responses.
	// <= 0 disables keep-alives. Value is in seconds.
	NonStreamKeepAliveInterval int `yaml:"nonstream-keepalive-interval,omitempty" json:"nonstream-keepalive-interval,omitempty"`

	// CodebuddyVision configures the CodeBuddy vision-proxy layer. When a chat
	// request carries image input for a model that does not natively support
	// images, the proxy either swaps the model to a vision model (routing) or
	// converts the images to text descriptions first (preprocess).
	CodebuddyVision CodebuddyVisionConfig `yaml:"codebuddy-vision" json:"codebuddy-vision"`
}

// ClaudeCodeConfig configures Claude Code compatibility behavior.
type ClaudeCodeConfig struct {
	// DisableCloakingModelList disables model ID cloaking in Anthropic model list responses.
	DisableCloakingModelList bool `yaml:"disable-cloaking-model-list" json:"disable-cloaking-model-list"`
}

// StreamingConfig holds server streaming behavior configuration.
type StreamingConfig struct {
	// KeepAliveSeconds controls how often the server emits SSE heartbeats (": keep-alive\n\n").
	// <= 0 disables keep-alives. Default is 0.
	KeepAliveSeconds int `yaml:"keepalive-seconds,omitempty" json:"keepalive-seconds,omitempty"`

	// BootstrapRetries controls how many times the server may retry a streaming request before any bytes are sent,
	// to allow auth rotation / transient recovery.
	// <= 0 disables bootstrap retries. Default is 0.
	BootstrapRetries          int `yaml:"bootstrap-retries,omitempty" json:"bootstrap-retries,omitempty"`
	StreamOpenMaxAttempts     int `yaml:"stream-open-max-attempts,omitempty" json:"stream-open-max-attempts,omitempty"`
	StreamOpenTimeoutMS       int `yaml:"stream-open-timeout-ms,omitempty" json:"stream-open-timeout-ms,omitempty"`
	StreamIdleTimeoutMS       int `yaml:"stream-idle-timeout-ms,omitempty" json:"stream-idle-timeout-ms,omitempty"`
	ImageStreamOpenTimeoutMS  int `yaml:"image-stream-open-timeout-ms,omitempty" json:"image-stream-open-timeout-ms,omitempty"`
	ImageStreamIdleTimeoutMS  int `yaml:"image-stream-idle-timeout-ms,omitempty" json:"image-stream-idle-timeout-ms,omitempty"`
	BootstrapRetryBaseDelayMS int `yaml:"bootstrap-retry-base-delay-ms,omitempty" json:"bootstrap-retry-base-delay-ms,omitempty"`
	BootstrapRetryMaxDelayMS  int `yaml:"bootstrap-retry-max-delay-ms,omitempty" json:"bootstrap-retry-max-delay-ms,omitempty"`
}

// CodebuddyVisionConfig controls the CodeBuddy vision-proxy layer.
//
// The Tencent CodeBuddy backend accepts image input on a per-model basis. Some
// text-only models (e.g. hunyuan-2.0-instruct) silently ignore images and reply
// with "this model does not support image input" instead of an error. When
// enabled, the proxy detects image input and handles it for non-vision models.
type CodebuddyVisionConfig struct {
	// Mode selects the strategy:
	//   - "off" (default): disabled; images pass through unchanged.
	//   - "routing": swap the request model to Model for non-vision models.
	//   - "preprocess": describe images with Model first, then continue with the
	//     original model.
	//   - "agentic": inject an inspect_image tool and run a server-side tool-calling
	//     loop so the text-only model can autonomously query the vision model
	//     multiple times during reasoning.
	// Any other value falls back to "off".
	Mode string `yaml:"mode" json:"mode"`

	// Model is the vision model used as the routing target / preprocess engine.
	// Default "hy3-preview".
	Model string `yaml:"model" json:"model"`

	// PreprocessPrompt overrides the user-visible prompt sent to the vision model
	// in preprocess mode. Empty uses a built-in default.
	PreprocessPrompt string `yaml:"preprocess-prompt" json:"preprocess-prompt"`

	// MaxToolRounds caps the number of inspect_image tool-call iterations in
	// agentic mode. Non-positive falls back to a default of 3.
	MaxToolRounds int `yaml:"max-tool-rounds" json:"max-tool-rounds"`
}

// VisionMode constants for CodebuddyVisionConfig.Mode.
const (
	CodebuddyVisionModeOff         = "off"
	CodebuddyVisionModeRouting     = "routing"
	CodebuddyVisionModePreprocess  = "preprocess"
	CodebuddyVisionModeAgentic     = "agentic"
)

// NormalizedVisionMode returns the effective mode, mapping unknown values to "off".
func (c CodebuddyVisionConfig) NormalizedVisionMode() string {
	switch strings.ToLower(strings.TrimSpace(c.Mode)) {
	case CodebuddyVisionModeRouting:
		return CodebuddyVisionModeRouting
	case CodebuddyVisionModePreprocess:
		return CodebuddyVisionModePreprocess
	case CodebuddyVisionModeAgentic:
		return CodebuddyVisionModeAgentic
	default:
		return CodebuddyVisionModeOff
	}
}

// MaxVisionToolRounds returns the effective agentic iteration cap (default 3).
func (c CodebuddyVisionConfig) MaxVisionToolRounds() int {
	if c.MaxToolRounds > 0 {
		return c.MaxToolRounds
	}
	return 3
}

// VisionModel returns the configured vision model, defaulting to "hy3-preview".
func (c CodebuddyVisionConfig) VisionModel() string {
	model := strings.TrimSpace(c.Model)
	if model == "" {
		return "hy3-preview"
	}
	return model
}

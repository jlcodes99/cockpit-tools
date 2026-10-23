package auth

import (
	"bytes"
	"context"
	"encoding/json"
	"strings"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

const continuityPreferredAuthMetadataKey = "continuity_preferred_auth"

const continuityRequireCodexMetadataKey = "continuity_require_codex"

func (m *Manager) prepareManagerExecutionContinuity(ctx context.Context, providers []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (context.Context, cliproxyexecutor.Options, error) {
	return prepareExecutionContinuity(ctx, providers, req, opts)
}

func continuityExecutionProviders(ctx context.Context, providers []string, opts cliproxyexecutor.Options) []string {
	if cliproxyexecutor.ContinuityPreparationError(ctx) != nil {
		filtered := make([]string, 0, len(providers))
		for _, provider := range providers {
			if provider != "codex" {
				filtered = append(filtered, provider)
			}
		}
		return filtered
	}
	if required, _ := opts.Metadata[continuityRequireCodexMetadataKey].(bool); required {
		return []string{"codex"}
	}
	return providers
}

func continuityOpaqueInput(value gjson.Result) bool {
	if !value.IsObject() && !value.IsArray() {
		return false
	}
	found := false
	value.ForEach(func(key, item gjson.Result) bool {
		if key.String() == "encrypted_content" && item.Type == gjson.String && item.String() != "" || continuityOpaqueInput(item) {
			found = true
			return false
		}
		return true
	})
	return found
}

func prepareExecutionContinuity(ctx context.Context, providers []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (context.Context, cliproxyexecutor.Options, error) {
	if cliproxyexecutor.ContinuityHookFromContext(ctx) == nil {
		return ctx, opts, nil
	}
	hasCodex, mixed := false, false
	for _, provider := range providers {
		if provider == "codex" {
			hasCodex = true
		} else {
			mixed = true
		}
	}
	if !hasCodex {
		return ctx, opts, nil
	}
	from := opts.SourceFormat
	if from == "" {
		from = req.Format
	}
	if from != sdktranslator.FormatCodex && from != sdktranslator.FormatOpenAIResponse && !sdktranslator.HasRequestTransformer(from, sdktranslator.FormatCodex) {
		return ctx, opts, nil
	}
	body := sdktranslator.TranslateRequest(from, sdktranslator.FormatCodex, req.Model, bytes.Clone(req.Payload), opts.Stream)
	body = cliproxyexecutor.NormalizeResponsesCallIDs(body, true)
	if !json.Valid(body) {
		return ctx, opts, &Error{Code: "continuity_invalid_payload", Message: "invalid normalized Responses request"}
	}
	preparedCtx, prepared, err := cliproxyexecutor.PrepareContinuity(ctx, cliproxyexecutor.ContinuityPrepareRequest{Headers: opts.Headers.Clone(), Body: body, Compact: opts.Alt == "responses/compact", AllowRebuild: true, SourceFormat: from.String(), Model: req.Model})
	if err != nil {
		if mixed && !continuityOwnershipError(err) {
			return cliproxyexecutor.WithContinuityPreparationError(ctx, err), opts, nil
		}
		return ctx, opts, err
	}
	requireCodex := mixed && (prepared.RequireCodex || prepared.PreferredAccountID != "" || gjson.GetBytes(body, "previous_response_id").String() != "" || continuityOpaqueInput(gjson.GetBytes(body, "input")))
	if prepared.PreferredAccountID != "" || requireCodex {
		metadata := make(map[string]any, len(opts.Metadata)+1)
		for key, value := range opts.Metadata {
			metadata[key] = value
		}
		if requireCodex {
			metadata[continuityRequireCodexMetadataKey] = true
		}
		if prepared.PreferredAccountID != "" && !prepared.SelfContained {
			metadata[cliproxyexecutor.PinnedAuthMetadataKey] = prepared.PreferredAccountID
		} else if prepared.PreferredAccountID != "" && pinnedAuthIDFromMetadata(metadata) == "" {
			metadata[continuityPreferredAuthMetadataKey] = prepared.PreferredAccountID
		}
		opts.Metadata = metadata
	}
	return preparedCtx, opts, nil
}

// A native fallback cannot authorize history owned by another caller or replace
// a state-dependent request with a stateless send to a different provider.
func continuityOwnershipError(err error) bool {
	code := gjson.Get(err.Error(), "error.code").String()
	switch code {
	case "upstream_state_caller_mismatch", "upstream_state_owner_changed", "upstream_state_owner_conflict",
		"upstream_turn_state_mismatch", "mixed_upstream_state", "mixed_upstream_generation",
		"account_binding_unavailable", "continuity_account_scope", "continuation_history_incomplete", "previous_response_not_found":
		return true
	}
	return false
}

func continuityPreferredAuth(opts cliproxyexecutor.Options, tried map[string]struct{}) string {
	preferred, _ := opts.Metadata[continuityPreferredAuthMetadataKey].(string)
	if _, ok := tried[preferred]; ok {
		return ""
	}
	return strings.TrimSpace(preferred)
}

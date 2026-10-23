package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

// Generate with COCKPIT_PARITY22_POLICY_OUT; ordinary test runs never rewrite fixtures.
// Every expected value comes from the existing Go policy/handler helpers.
func TestParity22PolicyReference(t *testing.T) {
	patterns := [][2]string{
		{"GPT-6.1-*", "gpt-6.1-sol"}, {"Vendor/*", "vendor/model"},
		{" gPt-image-* ", " GPT-IMAGE-2.5 "}, {"ΟΣ", "οσ"}, {"İ", "i"},
		{"gpt-*", "evil/gpt-6"}, {"gpt-6", "gpt-6-mini"},
		{"gpt-*-codex", "gpt-6-codex"}, {"", ""}, {"*", ""},
	}
	var rules []map[string]any
	for _, pair := range patterns {
		rules = append(rules, map[string]any{"pattern": pair[0], "model": pair[1], "matches": wildcardModelMatches(pair[0], pair[1])})
	}
	var catalogs []map[string]any
	for _, fixture := range []struct {
		name       string
		enabled    bool
		provider   bool
		namespaced bool
	}{{"off", false, false, false}, {"on", true, false, false}, {"provider", true, true, false}, {"namespace", true, false, true}} {
		spec := &apiKeySpec{ResponsesWebsockets: fixture.enabled && !fixture.namespaced}
		if fixture.provider {
			spec.ProviderGateway = &providerGatewaySpec{UpstreamModels: []string{"gpt-6.1-sol"}, UpstreamModel: "gpt-6.1-sol"}
		}
		response := buildCodexClientModelsResponse([]string{"gpt-6.1-sol"}, spec, nil, &manifest{})
		encoded, err := json.Marshal(response)
		if err != nil {
			t.Fatal(err)
		}
		var parsed struct {
			Models []struct {
				PreferWebsockets bool `json:"prefer_websockets"`
			} `json:"models"`
		}
		if err = json.Unmarshal(encoded, &parsed); err != nil || len(parsed.Models) != 1 {
			t.Fatalf("catalog %s: %s / %v", fixture.name, encoded, err)
		}
		catalogs = append(catalogs, map[string]any{"name": fixture.name, "enabled": fixture.enabled, "provider": fixture.provider, "namespaced": fixture.namespaced, "prefer": parsed.Models[0].PreferWebsockets})
	}
	const upstream = "vendor/private-extra-long-model-name"
	var counts []map[string]any
	for _, mode := range []string{"plain_profile", "provider", "automatic", "explicit_oauth_alias"} {
		alias, source := "short-alias", upstream
		if mode == "explicit_oauth_alias" {
			alias, source = "m", "gpt-6.1-sol"
		}
		m := &manifest{ModelIDs: []string{source}, aliasToSource: map[string]string{alias: source}}
		spec := &apiKeySpec{}
		if mode == "provider" {
			spec.ProviderGateway = &providerGatewaySpec{UpstreamModels: []string{upstream}, UpstreamModel: upstream}
		}
		if mode == "automatic" {
			spec.ModelRouting = &modelRoutingSpec{Automatic: true, NativeModels: []string{"short-alias"}}
		}
		if mode == "explicit_oauth_alias" {
			spec.ModelRouting = &modelRoutingSpec{DefaultRoute: "oauth", FailurePolicy: "strict"}
		}
		for _, messages := range []any{[]any{}, []any{map[string]any{"role": "user", "content": "hello world"}}} {
			input := map[string]any{"model": alias, "messages": messages}
			body, _ := json.Marshal(input)
			body, err := injectRequestBodyModelAndStream(body, alias, nil)
			if err != nil {
				t.Fatal(err)
			}
			rewritten, _, err := rewriteBodyModel(m, spec, "text", body)
			if err != nil {
				t.Fatal(err)
			}
			if rewritten != nil {
				body = rewritten
			}
			var canonical map[string]any
			_ = json.Unmarshal(body, &canonical)
			counts = append(counts, map[string]any{"mode": mode, "alias": alias, "upstream": source, "input": input, "canonical": canonical, "count": estimateRequestTokens(body)})
		}
	}
	result := map[string]any{"source": "Go production helpers; regenerate with TestParity22PolicyReference", "rules": rules, "catalogs": catalogs, "counts": counts}
	encoded, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if output := os.Getenv("COCKPIT_PARITY22_POLICY_OUT"); output != "" {
		if err = os.MkdirAll(filepath.Dir(output), 0o755); err != nil {
			t.Fatal(err)
		}
		if err = os.WriteFile(output, append(encoded, '\n'), 0o644); err != nil {
			t.Fatal(err)
		}
	} else {
		path := filepath.Join("..", "..", "src-tauri", "src", "modules", "codex_native_api", "fixtures", "parity22_policy_go.json")
		stored, errRead := os.ReadFile(path)
		if errRead != nil {
			t.Fatal(errRead)
		}
		var actual, expected any
		if err = json.Unmarshal(encoded, &actual); err != nil {
			t.Fatal(err)
		}
		if err = json.Unmarshal(stored, &expected); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(actual, expected) {
			t.Fatalf("Go policy behavior differs from %s; regenerate explicitly and inspect the differential", path)
		}
	}
}

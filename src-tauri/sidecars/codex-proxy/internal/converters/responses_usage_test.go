package converters

import "testing"

func TestExtractUsageMetrics_ClaudeTTLOnlyFields(t *testing.T) {
	usageRaw := map[string]interface{}{
		"input_tokens":                   100,
		"output_tokens":                  20,
		"cache_read_input_tokens":        30,
		"cache_creation_5m_input_tokens": 7,
		"cache_creation_1h_input_tokens": 3,
	}

	usage := ExtractUsageMetrics(usageRaw)
	if usage.TotalTokens != 160 {
		t.Fatalf("TotalTokens = %d, want 160", usage.TotalTokens)
	}
	if usage.CacheTTL != "mixed" {
		t.Fatalf("CacheTTL = %q, want mixed", usage.CacheTTL)
	}
	if usage.CacheCreation5mInputTokens != 7 || usage.CacheCreation1hInputTokens != 3 {
		t.Fatalf("cache ttl split mismatch: %#v", usage)
	}
}

func TestExtractUsageMetrics_ClaudeCacheCreationUsesAggregateWhenPresent(t *testing.T) {
	usageRaw := map[string]interface{}{
		"input_tokens":                   100,
		"output_tokens":                  20,
		"cache_read_input_tokens":        30,
		"cache_creation_input_tokens":    5,
		"cache_creation_5m_input_tokens": 7,
		"cache_creation_1h_input_tokens": 3,
	}

	usage := ExtractUsageMetrics(usageRaw)
	// 当 aggregate 字段存在时，优先使用 cache_creation_input_tokens，避免与 5m/1h 重复累计。
	if usage.TotalTokens != 155 {
		t.Fatalf("TotalTokens = %d, want 155", usage.TotalTokens)
	}
}

func TestOpenAIChatResponseToResponses_OpenAICacheDetailsNormalizesInput(t *testing.T) {
	openaiResp := map[string]interface{}{
		"id":      "chatcmpl-openai-cache",
		"model":   "gpt-5.5",
		"choices": []interface{}{map[string]interface{}{"message": map[string]interface{}{"content": "ok"}}},
		"usage": map[string]interface{}{
			"prompt_tokens":     38451,
			"completion_tokens": 1275,
			"total_tokens":      39726,
			"prompt_tokens_details": map[string]interface{}{
				"cached_tokens": 36608,
			},
		},
	}

	resp, err := OpenAIChatResponseToResponses(openaiResp, "sess_test")
	if err != nil {
		t.Fatalf("OpenAIChatResponseToResponses() error = %v", err)
	}
	if resp.Usage.InputTokens != 1843 {
		t.Fatalf("InputTokens = %d, want 1843", resp.Usage.InputTokens)
	}
	if resp.Usage.TotalTokens != 39726 {
		t.Fatalf("TotalTokens = %d, want 39726", resp.Usage.TotalTokens)
	}
	if resp.Usage.CacheReadInputTokens != 0 {
		t.Fatalf("CacheReadInputTokens = %d, want 0 for OpenAI cache details", resp.Usage.CacheReadInputTokens)
	}
	if resp.Usage.InputTokensDetails == nil || resp.Usage.InputTokensDetails.CachedTokens != 36608 {
		t.Fatalf("InputTokensDetails = %#v, want cached_tokens 36608", resp.Usage.InputTokensDetails)
	}
}

func TestExtractUsageMetrics_OpenAIResponsesKeepsPromptTotal(t *testing.T) {
	usageRaw := map[string]interface{}{
		"input_tokens":  38451,
		"output_tokens": 1275,
		"input_tokens_details": map[string]interface{}{
			"cached_tokens": 36608,
		},
	}

	usage := ExtractUsageMetrics(usageRaw)
	if usage.InputTokens != 38451 {
		t.Fatalf("InputTokens = %d, want raw responses total 38451", usage.InputTokens)
	}
	if usage.TotalTokens != 39726 {
		t.Fatalf("TotalTokens = %d, want 39726", usage.TotalTokens)
	}
	if usage.InputTokensDetails == nil || usage.InputTokensDetails.CachedTokens != 36608 {
		t.Fatalf("InputTokensDetails = %#v, want cached_tokens 36608", usage.InputTokensDetails)
	}
}

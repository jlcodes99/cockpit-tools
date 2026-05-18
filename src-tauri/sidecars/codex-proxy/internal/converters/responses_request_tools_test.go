package converters

import (
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/stretchr/testify/assert"
)

func TestResponsesToGeminiRequest_PreservesTools(t *testing.T) {
	req := &types.ResponsesRequest{
		Model: "gpt-5",
		Input: "hello",
		Tools: []map[string]interface{}{
			{
				"type":        "function",
				"name":        "get_weather",
				"description": "Get weather",
				"parameters": map[string]interface{}{
					"type": "object",
				},
			},
		},
	}

	geminiReq, err := ResponsesToGeminiRequest(&session.Session{}, req, "gemini-2.5-pro")
	assert.NoError(t, err)
	assert.Len(t, geminiReq.Tools, 1)
	assert.Len(t, geminiReq.Tools[0].FunctionDeclarations, 1)
	assert.Equal(t, "get_weather", geminiReq.Tools[0].FunctionDeclarations[0].Name)
}

func TestOpenAIChatConverter_ToProviderRequest_PreservesTools(t *testing.T) {
	parallelToolCalls := true
	req := &types.ResponsesRequest{
		Model:             "gpt-5",
		Input:             "hello",
		ToolChoice:        "auto",
		ParallelToolCalls: &parallelToolCalls,
		RawTools: []interface{}{
			map[string]interface{}{
				"type":        "function",
				"name":        "get_weather",
				"description": "Get weather",
				"parameters": map[string]interface{}{
					"type": "object",
				},
			},
		},
	}

	converted, err := (&OpenAIChatConverter{}).ToProviderRequest(&session.Session{}, req)
	assert.NoError(t, err)

	requestMap, ok := converted.(map[string]interface{})
	assert.True(t, ok)
	tools, ok := requestMap["tools"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, tools, 1)
	assert.Equal(t, "function", tools[0]["type"])
	assert.Equal(t, "auto", requestMap["tool_choice"])
	assert.Equal(t, true, requestMap["parallel_tool_calls"])
}

func TestClaudeConverter_ToProviderRequest_PreservesTools(t *testing.T) {
	req := &types.ResponsesRequest{
		Model: "claude-3-5-sonnet",
		Input: "hello",
		Tools: []map[string]interface{}{
			{
				"type":        "function",
				"name":        "get_weather",
				"description": "Get weather",
				"parameters": map[string]interface{}{
					"type": "object",
				},
			},
		},
	}

	converted, err := (&ClaudeConverter{}).ToProviderRequest(&session.Session{}, req)
	assert.NoError(t, err)

	requestMap, ok := converted.(map[string]interface{})
	assert.True(t, ok)
	tools, ok := requestMap["tools"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, tools, 1)
	assert.Equal(t, "get_weather", tools[0]["name"])
	assert.NotNil(t, tools[0]["input_schema"])
}

func TestOpenAIChatConverter_ToolRequiredField_FilledWhenMissing(t *testing.T) {
	req := &types.ResponsesRequest{
		Model: "gpt-5.5",
		Input: "list resources",
		RawTools: []interface{}{
			map[string]interface{}{
				"type":        "function",
				"name":        "list_mcp_resources",
				"description": "Lists resources",
				"parameters": map[string]interface{}{
					"type":                 "object",
					"properties":           map[string]interface{}{},
					"additionalProperties": false,
				},
			},
		},
	}

	converted, err := (&OpenAIChatConverter{}).ToProviderRequest(&session.Session{}, req)
	assert.NoError(t, err)
	requestMap := converted.(map[string]interface{})
	tools := requestMap["tools"].([]map[string]interface{})

	params, ok := tools[0]["function"].(map[string]interface{})["parameters"].(map[string]interface{})
	assert.True(t, ok)
	required, exists := params["required"]
	assert.True(t, exists, "required 字段必须存在")
	_, isArray := required.([]interface{})
	assert.True(t, isArray, "required 字段必须是数组")
	assert.False(t, exists && !isArray, "required 不应为 None 或非数组")
}

func TestOpenAIChatConverter_SkipsNonFunctionTools(t *testing.T) {
	req := &types.ResponsesRequest{
		Model: "gpt-5.5",
		Input: "search",
		RawTools: []interface{}{
			map[string]interface{}{"type": "web_search", "search_content_types": []interface{}{"text"}},
			map[string]interface{}{"type": "custom", "name": "apply_patch", "format": map[string]interface{}{"type": "grammar"}},
			map[string]interface{}{
				"type":       "function",
				"name":       "do_thing",
				"parameters": map[string]interface{}{"type": "object", "properties": map[string]interface{}{}},
			},
		},
	}

	converted, err := (&OpenAIChatConverter{}).ToProviderRequest(&session.Session{}, req)
	assert.NoError(t, err)
	requestMap := converted.(map[string]interface{})
	tools := requestMap["tools"].([]map[string]interface{})
	assert.Len(t, tools, 1, "只应保留 function 类型工具")
	assert.Equal(t, "do_thing", tools[0]["function"].(map[string]interface{})["name"])
}

func TestOpenAIChatConverter_PreservesStringToolsWhenCodexCompatDisabled(t *testing.T) {
	req := &types.ResponsesRequest{
		Model:    "gpt-5.5",
		Input:    "search",
		RawTools: []interface{}{"web_search_preview"},
	}

	converted, err := (&OpenAIChatConverter{}).ToProviderRequest(&session.Session{}, req)
	assert.NoError(t, err)
	requestMap := converted.(map[string]interface{})
	tools := requestMap["tools"].([]map[string]interface{})
	assert.Len(t, tools, 1)
	assert.Equal(t, "web_search_preview", tools[0]["function"].(map[string]interface{})["name"])
}

func TestOpenAIChatResponseToResponses_ToolCalls(t *testing.T) {
	openaiResp := map[string]interface{}{
		"model": "gpt-4o",
		"choices": []interface{}{
			map[string]interface{}{
				"message": map[string]interface{}{
					"role":    "assistant",
					"content": "I'll call a tool.",
					"tool_calls": []interface{}{
						map[string]interface{}{
							"id":   "call_123",
							"type": "function",
							"function": map[string]interface{}{
								"name":      "get_weather",
								"arguments": `{"location":"NYC"}`,
							},
						},
					},
				},
			},
		},
	}

	resp, err := OpenAIChatResponseToResponses(openaiResp, "sess_test")
	assert.NoError(t, err)
	assert.Len(t, resp.Output, 2)
	assert.Equal(t, "message", resp.Output[0].Type)
	assert.Equal(t, "function_call", resp.Output[1].Type)
	assert.Equal(t, "call_123", resp.Output[1].CallID)
	assert.Equal(t, "get_weather", resp.Output[1].Name)
}

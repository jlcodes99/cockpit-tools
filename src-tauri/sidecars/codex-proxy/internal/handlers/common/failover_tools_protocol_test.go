package common

import "testing"

func TestShouldRetryWithNextKey_ResponsesToolsProtocolError(t *testing.T) {
	body := []byte(`{"error":{"message":"Missing required parameter: 'tools[15].tools'.","type":"invalid_request_error","param":"tools[15].tools","code":"missing_required_parameter"}}`)

	t.Run("fuzzy 模式允许 failover", func(t *testing.T) {
		gotFailover, _ := ShouldRetryWithNextKey(400, body, true, "Responses")
		if !gotFailover {
			t.Fatalf("tools 协议兼容 400 应允许 failover，当前被拦截")
		}
	})

	t.Run("仅返回 tools param 也允许 failover", func(t *testing.T) {
		body := []byte(`{"error":{"message":"Invalid schema for function 'list_mcp_resources': None is not of type 'array'.","type":"invalid_request_error","param":"tools","code":"invalid_function_parameters"}}`)
		gotFailover, _ := ShouldRetryWithNextKey(400, body, true, "Responses")
		if !gotFailover {
			t.Fatalf("param=tools 的协议兼容 400 应允许 failover")
		}
	})

	t.Run("tools 点路径也允许 failover", func(t *testing.T) {
		body := []byte(`{"error":{"message":"invalid schema: expected object","type":"invalid_request_error","param":"tools.0","code":"invalid_request_error"}}`)
		gotFailover, _ := ShouldRetryWithNextKey(400, body, true, "Responses")
		if !gotFailover {
			t.Fatalf("param=tools.0 的协议兼容 400 应允许 failover")
		}
	})

	t.Run("非 Responses 接口仍按 schema 错误处理", func(t *testing.T) {
		if !isNonRetryableError(body, "Messages") {
			t.Fatalf("非 Responses 接口不应放行 tools schema 错误")
		}
	})
}

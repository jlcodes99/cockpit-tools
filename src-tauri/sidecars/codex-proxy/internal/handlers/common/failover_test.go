package common

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gin-gonic/gin"
)

// TestClassifyByStatusCode 测试基于状态码的分类
func TestClassifyByStatusCode(t *testing.T) {
	tests := []struct {
		name         string
		statusCode   int
		wantFailover bool
		wantQuota    bool
	}{
		// 认证/授权错误
		{"401 Unauthorized", 401, true, false},
		{"403 Forbidden", 403, true, false},

		// 配额/计费错误
		{"402 Payment Required", 402, true, true},
		{"429 Too Many Requests", 429, true, true},

		// 超时错误
		{"408 Request Timeout", 408, true, false},

		// 服务端错误
		{"500 Internal Server Error", 500, true, false},
		{"502 Bad Gateway", 502, true, false},
		{"503 Service Unavailable", 503, true, false},
		{"504 Gateway Timeout", 504, true, false},

		// 不应 failover 的客户端错误
		{"400 Bad Request", 400, false, false},
		{"404 Not Found", 404, false, false},
		{"405 Method Not Allowed", 405, false, false},
		{"413 Payload Too Large", 413, false, false},
		{"422 Unprocessable Entity", 422, false, false},

		// 成功状态码
		{"200 OK", 200, false, false},
		{"201 Created", 201, false, false},
		{"204 No Content", 204, false, false},

		// 重定向
		{"301 Moved Permanently", 301, false, false},
		{"302 Found", 302, false, false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotFailover, gotQuota := classifyByStatusCode(tt.statusCode)
			if gotFailover != tt.wantFailover {
				t.Errorf("classifyByStatusCode(%d) failover = %v, want %v", tt.statusCode, gotFailover, tt.wantFailover)
			}
			if gotQuota != tt.wantQuota {
				t.Errorf("classifyByStatusCode(%d) quota = %v, want %v", tt.statusCode, gotQuota, tt.wantQuota)
			}
		})
	}
}

// TestClassifyMessage 测试基于错误消息的分类
func TestClassifyMessage(t *testing.T) {
	tests := []struct {
		name         string
		message      string
		wantFailover bool
		wantQuota    bool
	}{
		// 配额相关
		{"insufficient credits", "You have insufficient credits", true, true},
		{"quota exceeded", "API quota exceeded for this month", true, true},
		{"rate limit", "Rate limit exceeded, please retry later", true, true},
		{"balance", "Account balance is zero", true, true},
		{"billing", "Billing issue detected", true, true},
		{"中文-积分不足", "您的积分不足，请充值", true, true},
		{"中文-余额不足", "账户余额不足", true, true},
		{"中文-请求数限制", "已达到请求数限制", true, true},

		// 认证相关
		{"invalid api key", "Invalid API key provided", true, false},
		{"unauthorized", "Unauthorized access", true, false},
		{"token expired", "Your token has expired", true, false},
		{"permission denied", "Permission denied for this resource", true, false},
		{"中文-密钥无效", "密钥无效，请检查", true, false},
		{"中文-令牌已过期", "该令牌已过期", true, false},

		// 临时错误
		{"timeout", "Request timeout, please retry", true, false},
		{"server overloaded", "Server is overloaded", true, false},
		{"temporarily unavailable", "Service temporarily unavailable", true, false},
		{"中文-超时", "请求超时", true, false},

		// 不应 failover
		{"normal error", "Something went wrong", false, false},
		{"validation error", "Field 'name' is required", false, false},
		{"schema invalid value", "Invalid value: 'input_text'. Supported values are: 'output_text' and 'refusal'.", false, false},
		{"empty message", "", false, false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotFailover, gotQuota := classifyMessage(tt.message)
			if gotFailover != tt.wantFailover {
				t.Errorf("classifyMessage(%q) failover = %v, want %v", tt.message, gotFailover, tt.wantFailover)
			}
			if gotQuota != tt.wantQuota {
				t.Errorf("classifyMessage(%q) quota = %v, want %v", tt.message, gotQuota, tt.wantQuota)
			}
		})
	}
}

// TestClassifyErrorType 测试基于错误类型的分类
func TestClassifyErrorType(t *testing.T) {
	tests := []struct {
		name         string
		errType      string
		wantFailover bool
		wantQuota    bool
	}{
		// 配额相关
		{"over_quota", "over_quota", true, true},
		{"quota_exceeded", "quota_exceeded", true, true},
		{"rate_limit_exceeded", "rate_limit_exceeded", true, true},
		{"billing_error", "billing_error", true, true},
		{"insufficient_funds", "insufficient_funds", true, true},

		// 认证相关
		{"authentication_error", "authentication_error", true, false},
		{"invalid_api_key", "invalid_api_key", true, false},
		{"permission_denied", "permission_denied", true, false},

		// 服务端错误
		{"server_error", "server_error", true, false},
		{"internal_error", "internal_error", true, false},
		{"service_unavailable", "service_unavailable", true, false},

		// 不应 failover
		{"invalid_request", "invalid_request", false, false},
		{"validation_error", "validation_error", false, false},
		{"unknown_error", "unknown_error", false, false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotFailover, gotQuota := classifyErrorType(tt.errType)
			if gotFailover != tt.wantFailover {
				t.Errorf("classifyErrorType(%q) failover = %v, want %v", tt.errType, gotFailover, tt.wantFailover)
			}
			if gotQuota != tt.wantQuota {
				t.Errorf("classifyErrorType(%q) quota = %v, want %v", tt.errType, gotQuota, tt.wantQuota)
			}
		})
	}
}

// TestClassifyByErrorMessage 测试基于响应体的分类
func TestClassifyByErrorMessage(t *testing.T) {
	tests := []struct {
		name         string
		body         map[string]interface{}
		wantFailover bool
		wantQuota    bool
	}{
		{
			name: "quota error in message",
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"message": "You have exceeded your quota",
					"type":    "error",
				},
			},
			wantFailover: true,
			wantQuota:    true,
		},
		{
			name: "auth error in message",
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"message": "Invalid API key",
					"type":    "error",
				},
			},
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name: "quota error in type",
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"message": "Error occurred",
					"type":    "over_quota",
				},
			},
			wantFailover: true,
			wantQuota:    true,
		},
		{
			name: "server error in type",
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"message": "Error occurred",
					"type":    "server_error",
				},
			},
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name: "no failover keywords",
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"message": "Bad request format",
					"type":    "invalid_request",
				},
			},
			wantFailover: false,
			wantQuota:    false,
		},
		{
			name: "schema invalid value in message",
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"message": "Invalid value: 'input_text'. Supported values are: 'output_text' and 'refusal'.",
					"type":    "invalid_request_error",
				},
			},
			wantFailover: false,
			wantQuota:    false,
		},
		{
			name:         "empty body",
			body:         map[string]interface{}{},
			wantFailover: false,
			wantQuota:    false,
		},
		{
			name: "no error field",
			body: map[string]interface{}{
				"status": "error",
			},
			wantFailover: false,
			wantQuota:    false,
		},
		// upstream_error 字段支持（Responses API 错误格式）
		{
			name: "upstream_error string field - auth error",
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"type":           "upstream_error",
					"upstream_error": "Invalid API key provided",
				},
			},
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name: "upstream_error string field - quota error",
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"type":           "upstream_error",
					"upstream_error": "Rate limit exceeded, please retry later",
				},
			},
			wantFailover: true,
			wantQuota:    true,
		},
		{
			name: "upstream_error nested object with message",
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"type": "upstream_error",
					"upstream_error": map[string]interface{}{
						"message": "Insufficient credits",
					},
				},
			},
			wantFailover: true,
			wantQuota:    true,
		},
		{
			name: "detail field - auth error",
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"type":   "error",
					"detail": "Token expired, please refresh",
				},
			},
			wantFailover: true,
			wantQuota:    false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			bodyBytes, _ := json.Marshal(tt.body)
			gotFailover, gotQuota := classifyByErrorMessage(bodyBytes, "Messages")
			if gotFailover != tt.wantFailover {
				t.Errorf("classifyByErrorMessage() failover = %v, want %v", gotFailover, tt.wantFailover)
			}
			if gotQuota != tt.wantQuota {
				t.Errorf("classifyByErrorMessage() quota = %v, want %v", gotQuota, tt.wantQuota)
			}
		})
	}
}

// TestClassifyByErrorMessage_InvalidJSON 测试无效 JSON 的处理
func TestClassifyByErrorMessage_InvalidJSON(t *testing.T) {
	invalidBodies := [][]byte{
		[]byte("not json"),
		[]byte("{invalid}"),
		[]byte(""),
		nil,
	}

	for _, body := range invalidBodies {
		gotFailover, gotQuota := classifyByErrorMessage(body, "Messages")
		if gotFailover || gotQuota {
			t.Errorf("classifyByErrorMessage(%q) should return (false, false) for invalid JSON", string(body))
		}
	}
}

// TestShouldRetryWithNextKey_403WithPredeductQuotaError 测试 403 + 预扣费额度失败的场景
// 这是生产环境实际发生的错误格式
func TestShouldRetryWithNextKey_403WithPredeductQuotaError(t *testing.T) {
	// 使用生产环境的精确 JSON 格式
	body := []byte(`{"error":{"type":"new_api_error","message":"预扣费额度失败, 用户剩余额度: ¥0.053950, 需要预扣费额度: ¥0.191160, 下次重置时间: 2025-01-01 00:00:00"},"type":"error"}`)

	gotFailover, gotQuota := ShouldRetryWithNextKey(403, body, false, "Messages")

	if !gotFailover {
		t.Errorf("ShouldRetryWithNextKey(403, prededuct_error, false) failover = %v, want true", gotFailover)
	}
	if !gotQuota {
		t.Errorf("ShouldRetryWithNextKey(403, prededuct_error, false) quota = %v, want true", gotQuota)
	}
}

// TestClassifyMessage_ChineseQuotaKeywords 测试中文额度关键词
func TestClassifyMessage_ChineseQuotaKeywords(t *testing.T) {
	tests := []struct {
		name         string
		message      string
		wantFailover bool
		wantQuota    bool
	}{
		{"预扣费额度失败", "预扣费额度失败, 用户剩余额度: ¥0.053950", true, true},
		{"额度不足", "账户额度不足", true, true},
		{"预扣费失败", "预扣费失败，请充值", true, true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotFailover, gotQuota := classifyMessage(tt.message)
			if gotFailover != tt.wantFailover {
				t.Errorf("classifyMessage(%q) failover = %v, want %v", tt.message, gotFailover, tt.wantFailover)
			}
			if gotQuota != tt.wantQuota {
				t.Errorf("classifyMessage(%q) quota = %v, want %v", tt.message, gotQuota, tt.wantQuota)
			}
		})
	}
}

// TestShouldRetryWithNextKey 测试完整的重试判断逻辑
func TestShouldRetryWithNextKey(t *testing.T) {
	tests := []struct {
		name         string
		statusCode   int
		body         map[string]interface{}
		wantFailover bool
		wantQuota    bool
	}{
		// 403 + 中文配额相关消息
		{
			name:       "403 with chinese quota message",
			statusCode: 403,
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"type":    "new_api_error",
					"message": "预扣费额度失败, 用户剩余额度: ¥0.053950",
				},
				"type": "error",
			},
			wantFailover: true,
			wantQuota:    true,
		},
		// 状态码优先
		{
			name:         "401 always failover",
			statusCode:   401,
			body:         map[string]interface{}{},
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "402 always failover with quota",
			statusCode:   402,
			body:         map[string]interface{}{},
			wantFailover: true,
			wantQuota:    true,
		},
		{
			name:         "408 always failover",
			statusCode:   408,
			body:         map[string]interface{}{},
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "500 always failover",
			statusCode:   500,
			body:         map[string]interface{}{},
			wantFailover: true,
			wantQuota:    false,
		},
		// 400 需要检查消息体
		{
			name:       "400 with quota message",
			statusCode: 400,
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"message": "Quota exceeded",
				},
			},
			wantFailover: true,
			wantQuota:    true,
		},
		{
			name:       "400 with auth message",
			statusCode: 400,
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"message": "Invalid API key",
				},
			},
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:       "400 without failover keywords",
			statusCode: 400,
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"message": "Bad request",
				},
			},
			wantFailover: false,
			wantQuota:    false,
		},
		{
			name:       "400 invalid_request_error should not failover",
			statusCode: 400,
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"type":    "invalid_request_error",
					"message": "Invalid value: 'input_text'. Supported values are: 'output_text' and 'refusal'.",
				},
			},
			wantFailover: false,
			wantQuota:    false,
		},
		{
			name:       "400 anthropic thinking field required should not failover",
			statusCode: 400,
			body: map[string]interface{}{
				"error": map[string]interface{}{
					"type":    "invalid_request_error",
					"message": "messages.1213.content.0.thinking.thinking: Field required",
				},
			},
			wantFailover: false,
			wantQuota:    false,
		},
		// 404 不应 failover
		{
			name:         "404 never failover",
			statusCode:   404,
			body:         map[string]interface{}{},
			wantFailover: false,
			wantQuota:    false,
		},
		// 200 不应 failover
		{
			name:         "200 never failover",
			statusCode:   200,
			body:         map[string]interface{}{},
			wantFailover: false,
			wantQuota:    false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			bodyBytes, _ := json.Marshal(tt.body)
			// 测试非 Fuzzy 模式（精确错误分类）
			gotFailover, gotQuota := ShouldRetryWithNextKey(tt.statusCode, bodyBytes, false, "Messages")
			if gotFailover != tt.wantFailover {
				t.Errorf("shouldRetryWithNextKey(%d, ..., false) failover = %v, want %v", tt.statusCode, gotFailover, tt.wantFailover)
			}
			if gotQuota != tt.wantQuota {
				t.Errorf("shouldRetryWithNextKey(%d, ..., false) quota = %v, want %v", tt.statusCode, gotQuota, tt.wantQuota)
			}
		})
	}
}

func TestIsInsufficientBalanceMessage_HighConfidenceVariants(t *testing.T) {
	tests := []struct {
		name string
		msg  string
		want bool
	}{
		{name: "english insufficient credits", msg: "You have insufficient credits remaining", want: true},
		{name: "english out of credits", msg: "This account is out of credits", want: true},
		{name: "english no balance", msg: "no balance", want: true},
		{name: "english insufficient funds", msg: "payment declined: insufficient funds", want: true},
		{name: "english quota used up", msg: "quota used up for current billing period", want: true},
		{name: "english token quota not enough", msg: "token quota is not enough, token remain quota: ¥0.100000, need quota: ¥0.300000", want: true},
		{name: "english daily usage limit exceeded", msg: "daily usage limit exceeded", want: true},
		{name: "english daily limit exceeded", msg: "reason=\"DAILY_LIMIT_EXCEEDED\" message=\"daily usage limit exceeded\"", want: true},
		{name: "chinese balance exhausted", msg: "账户余额已用尽，请充值", want: true},
		{name: "chinese quota used up", msg: "账户额度已用完", want: true},
		{name: "chinese quota exhausted", msg: "当前额度耗尽", want: true},
		{name: "negative billing setup", msg: "billing not enabled for this account", want: false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := isInsufficientBalanceMessage(tt.msg)
			if got != tt.want {
				t.Fatalf("isInsufficientBalanceMessage(%q) = %v, want %v", tt.msg, got, tt.want)
			}
		})
	}
}

func TestShouldBlacklistKey_BalanceMessages(t *testing.T) {
	tests := []struct {
		name       string
		statusCode int
		body       string
		want       BlacklistResult
	}{
		{
			name:       "403 top level code insufficient balance should blacklist",
			statusCode: 403,
			body:       `{"code":"INSUFFICIENT_BALANCE","message":"Insufficient account balance"}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "insufficient_balance",
				Message:         "Insufficient account balance",
			},
		},
		{
			name:       "403 nested error code insufficient balance should blacklist",
			statusCode: 403,
			body:       `{"error":{"code":"INSUFFICIENT_BALANCE","message":"Insufficient account balance"}}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "insufficient_balance",
				Message:         "Insufficient account balance",
			},
		},
		{
			name:       "403 string error field with insufficient balance should blacklist",
			statusCode: 403,
			body:       `{"error":"API Key额度不足，请访问https://right.codes查看详情"}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "insufficient_balance",
				Message:         "API Key额度不足，请访问https://right.codes查看详情",
			},
		},
		{
			name:       "401 string error should still honor top level authentication type",
			statusCode: 401,
			body:       `{"error":"认证失败","type":"authentication_error"}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "authentication_error",
				Message:         "认证失败",
			},
		},
		{
			name:       "401 string error invalid api key without type should blacklist",
			statusCode: 401,
			body:       `{"error":"无效的API Key"}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "authentication_error",
				Message:         "无效的API Key",
			},
		},
		{
			name:       "401 new api token expired message should blacklist as authentication",
			statusCode: 401,
			body:       `{"error":{"code":"","message":"该令牌已过期 (request id: 202605041407066680249308268d9d6QnF3nAtC)","type":"new_api_error"}}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "authentication_error",
				Message:         "该令牌已过期 (request id: 202605041407066680249308268d9d6QnF3nAtC)",
			},
		},
		{
			name:       "403 top level insufficient account balance message should blacklist",
			statusCode: 403,
			body:       `{"message":"Insufficient account balance"}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "insufficient_balance",
				Message:         "Insufficient account balance",
			},
		},
		{
			name:       "403 prededuct quota message should blacklist as insufficient balance",
			statusCode: 403,
			body:       `{"error":{"type":"new_api_error","message":"预扣费额度失败, 用户剩余额度: ＄0.411202, 需要预扣费额度: ＄0.553368"},"type":"error"}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "insufficient_balance",
				Message:         "预扣费额度失败, 用户剩余额度: ＄0.411202, 需要预扣费额度: ＄0.553368",
			},
		},
		{
			name:       "403 token quota not enough message should blacklist as insufficient balance",
			statusCode: 403,
			body:       `{"error":{"message":"token quota is not enough, token remain quota: ¥0.100000, need quota: ¥0.300000 (request id: 20260426121858142194522mDUp325B)","type":"new_api_error","param":"","code":"pre_consume_quota_failed"},"type":"error"}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "insufficient_balance",
				Message:         "token quota is not enough, token remain quota: ¥0.100000, need quota: ¥0.300000 (request id: 20260426121858142194522mDUp325B)",
			},
		},
		{
			name:       "429 insufficient quota message should blacklist as insufficient balance",
			statusCode: 429,
			body:       `{"error":{"message":"insufficient quota for current billing period"}}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "insufficient_balance",
				Message:         "insufficient quota for current billing period",
			},
		},
		{
			name:       "429 top level usage limit exceeded code should blacklist as insufficient balance",
			statusCode: 429,
			body:       `{"code":"USAGE_LIMIT_EXCEEDED","message":"error: code=429 reason=\"DAILY_LIMIT_EXCEEDED\" message=\"daily usage limit exceeded\" metadata=map[]"}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "insufficient_balance",
				Message:         "error: code=429 reason=\"DAILY_LIMIT_EXCEEDED\" message=\"daily usage limit exceeded\" metadata=map[]",
			},
		},
		{
			name:       "429 nested daily limit exceeded code should blacklist as insufficient balance",
			statusCode: 429,
			body:       `{"error":{"code":"DAILY_LIMIT_EXCEEDED","message":"daily usage limit exceeded"}}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "insufficient_balance",
				Message:         "daily usage limit exceeded",
			},
		},
		{
			name:       "401 token status exhausted message should blacklist as insufficient balance",
			statusCode: 401,
			body:       `{"error":{"code":"","message":"该令牌额度已用尽 TokenStatusExhausted[sk-duK***qqX]"}}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "insufficient_balance",
				Message:         "该令牌额度已用尽 TokenStatusExhausted[sk-duK***qqX]",
			},
		},
		{
			name:       "401 out of credits message should blacklist as insufficient balance",
			statusCode: 401,
			body:       `{"error":{"message":"This account is out of credits"}}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "insufficient_balance",
				Message:         "This account is out of credits",
			},
		},
		{
			name:       "403 billing not enabled should not be misclassified as balance",
			statusCode: 403,
			body:       `{"error":{"message":"billing not enabled for this account"}}`,
			want:       BlacklistResult{},
		},
		{
			name:       "403 permission denied should not be misclassified as balance",
			statusCode: 403,
			body:       `{"error":{"type":"forbidden","message":"permission denied for this resource"}}`,
			want:       BlacklistResult{},
		},
		{
			name:       "403 explicit permission error should still be permission blacklist",
			statusCode: 403,
			body:       `{"error":{"type":"permission_denied","message":"permission denied"}}`,
			want: BlacklistResult{
				ShouldBlacklist: true,
				Reason:          "permission_error",
				Message:         "permission denied",
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := ShouldBlacklistKey(tt.statusCode, []byte(tt.body))
			if got != tt.want {
				t.Fatalf("ShouldBlacklistKey(%d, %s) = %+v, want %+v", tt.statusCode, tt.body, got, tt.want)
			}
		})
	}
}

func TestShouldRetryWithNextKey_TopLevelDetailAndAuthMessages(t *testing.T) {
	tests := []struct {
		name         string
		statusCode   int
		body         string
		fuzzyMode    bool
		wantFailover bool
		wantQuota    bool
	}{
		{
			name:         "top level detail not found remains non quota failover in fuzzy mode",
			statusCode:   404,
			body:         `{"detail":"Not Found"}`,
			fuzzyMode:    true,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "top level message chinese auth error",
			statusCode:   401,
			body:         `{"message":"身份验证失败。","type":"authentication_error"}`,
			fuzzyMode:    false,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "top level detail chinese invalid token",
			statusCode:   401,
			body:         `{"detail":"无效的令牌","type":"authentication_error"}`,
			fuzzyMode:    false,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "string error field auth message",
			statusCode:   401,
			body:         `{"error":"身份验证失败。"}`,
			fuzzyMode:    false,
			wantFailover: true,
			wantQuota:    false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotFailover, gotQuota := ShouldRetryWithNextKey(tt.statusCode, []byte(tt.body), tt.fuzzyMode, "Messages")
			if gotFailover != tt.wantFailover {
				t.Fatalf("ShouldRetryWithNextKey(%d, %s, %v) failover = %v, want %v", tt.statusCode, tt.body, tt.fuzzyMode, gotFailover, tt.wantFailover)
			}
			if gotQuota != tt.wantQuota {
				t.Fatalf("ShouldRetryWithNextKey(%d, %s, %v) quota = %v, want %v", tt.statusCode, tt.body, tt.fuzzyMode, gotQuota, tt.wantQuota)
			}
		})
	}
}

// TestShouldRetryWithNextKeyFuzzyMode 测试 Fuzzy 模式下的错误分类
// Fuzzy 模式：所有非 2xx 错误都触发 failover
func TestShouldRetryWithNextKeyFuzzyMode(t *testing.T) {
	tests := []struct {
		name         string
		statusCode   int
		wantFailover bool
		wantQuota    bool
	}{
		// 2xx 成功响应不 failover
		{
			name:         "200 OK - no failover",
			statusCode:   200,
			wantFailover: false,
			wantQuota:    false,
		},
		{
			name:         "201 Created - no failover",
			statusCode:   201,
			wantFailover: false,
			wantQuota:    false,
		},
		// 3xx 重定向在 Fuzzy 模式下触发 failover
		{
			name:         "301 Redirect - failover in fuzzy mode",
			statusCode:   301,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "302 Found - failover in fuzzy mode",
			statusCode:   302,
			wantFailover: true,
			wantQuota:    false,
		},
		// 4xx 客户端错误在 Fuzzy 模式下都触发 failover
		{
			name:         "400 Bad Request - failover in fuzzy mode",
			statusCode:   400,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "401 Unauthorized - failover in fuzzy mode",
			statusCode:   401,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "402 Payment Required - failover with quota",
			statusCode:   402,
			wantFailover: true,
			wantQuota:    true, // 配额相关
		},
		{
			name:         "403 Forbidden - failover in fuzzy mode",
			statusCode:   403,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "404 Not Found - failover in fuzzy mode",
			statusCode:   404,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "422 Unprocessable Entity - failover in fuzzy mode",
			statusCode:   422,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "429 Too Many Requests - failover with quota",
			statusCode:   429,
			wantFailover: true,
			wantQuota:    true, // 配额相关
		},
		// 5xx 服务端错误在 Fuzzy 模式下触发 failover
		{
			name:         "500 Internal Server Error - failover in fuzzy mode",
			statusCode:   500,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "502 Bad Gateway - failover in fuzzy mode",
			statusCode:   502,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "503 Service Unavailable - failover in fuzzy mode",
			statusCode:   503,
			wantFailover: true,
			wantQuota:    false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			// 测试 Fuzzy 模式（所有非 2xx 都 failover）
			gotFailover, gotQuota := ShouldRetryWithNextKey(tt.statusCode, nil, true, "Messages")
			if gotFailover != tt.wantFailover {
				t.Errorf("shouldRetryWithNextKey(%d, nil, true) failover = %v, want %v", tt.statusCode, gotFailover, tt.wantFailover)
			}
			if gotQuota != tt.wantQuota {
				t.Errorf("shouldRetryWithNextKey(%d, nil, true) quota = %v, want %v", tt.statusCode, gotQuota, tt.wantQuota)
			}
		})
	}
}

// TestShouldRetryWithNextKey_FuzzyMode_403WithQuotaMessage 测试 Fuzzy 模式下 403 + 预扣费消息
// 验证修复：Fuzzy 模式下也会检查消息体中的配额相关关键词
func TestShouldRetryWithNextKey_FuzzyMode_403WithQuotaMessage(t *testing.T) {
	tests := []struct {
		name         string
		statusCode   int
		body         []byte
		wantFailover bool
		wantQuota    bool
	}{
		{
			name:         "403 with prededuct quota error in fuzzy mode",
			statusCode:   403,
			body:         []byte(`{"error":{"type":"new_api_error","message":"预扣费额度失败, 用户剩余额度: ¥0.053950, 需要预扣费额度: ¥0.191160"},"type":"error"}`),
			wantFailover: true,
			wantQuota:    true,
		},
		{
			name:         "403 with insufficient balance in fuzzy mode",
			statusCode:   403,
			body:         []byte(`{"error":{"message":"余额不足，请充值"}}`),
			wantFailover: true,
			wantQuota:    true,
		},
		{
			name:         "403 without quota keywords in fuzzy mode",
			statusCode:   403,
			body:         []byte(`{"error":{"message":"Access denied"}}`),
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "403 with empty body in fuzzy mode",
			statusCode:   403,
			body:         nil,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "500 with quota message in fuzzy mode",
			statusCode:   500,
			body:         []byte(`{"error":{"message":"Quota exceeded"}}`),
			wantFailover: true,
			wantQuota:    true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotFailover, gotQuota := ShouldRetryWithNextKey(tt.statusCode, tt.body, true, "Messages")
			if gotFailover != tt.wantFailover {
				t.Errorf("ShouldRetryWithNextKey(%d, body, true) failover = %v, want %v", tt.statusCode, gotFailover, tt.wantFailover)
			}
			if gotQuota != tt.wantQuota {
				t.Errorf("ShouldRetryWithNextKey(%d, body, true) quota = %v, want %v", tt.statusCode, gotQuota, tt.wantQuota)
			}
		})
	}
}

func TestShouldRetryWithNextKey_FuzzyMode_InvalidRequestShouldNotFailover(t *testing.T) {
	tests := []struct {
		name string
		body []byte
	}{
		{
			name: "invalid_request_error type",
			body: []byte(`{"error":{"type":"invalid_request_error","message":"Invalid value: 'input_text'. Supported values are: 'output_text' and 'refusal'."}}`),
		},
		{
			name: "schema validation message in upstream_error",
			body: []byte(`{"error":{"type":"upstream_error","upstream_error":{"message":"Schema validation failed: unsupported content type input_text"}}}`),
		},
		{
			name: "anthropic thinking field required",
			body: []byte(`{"error":{"type":"invalid_request_error","message":"messages.1213.content.0.thinking.thinking: Field required"},"type":"error"}`),
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotFailover, gotQuota := ShouldRetryWithNextKey(400, tt.body, true, "Messages")
			if gotFailover {
				t.Errorf("ShouldRetryWithNextKey(400, invalid_request_body, true) failover = %v, want false", gotFailover)
			}
			if gotQuota {
				t.Errorf("ShouldRetryWithNextKey(400, invalid_request_body, true) quota = %v, want false", gotQuota)
			}
		})
	}
}

func TestShouldRetryWithNextKey_InvalidRequest5xxShouldFailover(t *testing.T) {
	tests := []struct {
		name      string
		body      []byte
		fuzzyMode bool
	}{
		{
			name:      "invalid_request code - normal mode",
			body:      []byte(`{"error":{"code":"invalid_request","message":"invalid request from upstream"}}`),
			fuzzyMode: false,
		},
		{
			name:      "invalid_request code - fuzzy mode",
			body:      []byte(`{"error":{"code":"invalid_request","message":"invalid request from upstream"}}`),
			fuzzyMode: true,
		},
		{
			name:      "schema validation message - normal mode",
			body:      []byte(`{"error":{"type":"upstream_error","upstream_error":{"message":"Schema validation failed: unsupported content type input_text"}}}`),
			fuzzyMode: false,
		},
		{
			name:      "schema validation message - fuzzy mode",
			body:      []byte(`{"error":{"type":"upstream_error","upstream_error":{"message":"Schema validation failed: unsupported content type input_text"}}}`),
			fuzzyMode: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotFailover, gotQuota := ShouldRetryWithNextKey(500, tt.body, tt.fuzzyMode, "Messages")
			if !gotFailover {
				t.Errorf("ShouldRetryWithNextKey(500, invalid_request_body, %v) failover = %v, want true", tt.fuzzyMode, gotFailover)
			}
			if gotQuota {
				t.Errorf("ShouldRetryWithNextKey(500, invalid_request_body, %v) quota = %v, want false", tt.fuzzyMode, gotQuota)
			}
		})
	}
}

// TestIsNonRetryableErrorCode 测试参数校验类不可重试错误码判断
func TestIsNonRetryableErrorCode(t *testing.T) {
	tests := []struct {
		code string
		want bool
	}{
		// 请求内容无效 - 不应重试
		{"invalid_request", true},
		{"invalid_request_error", true},
		{"bad_request", true},
		// 内容审核相关 - 已拆分到 isContentModerationErrorCode，此处应返回 false
		{"sensitive_words_detected", false},
		{"content_policy_violation", false},
		{"content_filter", false},
		{"content_blocked", false},
		{"moderation_blocked", false},
		// 其他错误码 - 应该重试
		{"server_error", false},
		{"rate_limit", false},
		{"authentication_error", false},
		{"unknown_error", false},
		{"", false},
	}

	for _, tt := range tests {
		name := tt.code
		if name == "" {
			name = "empty"
		}
		t.Run(name, func(t *testing.T) {
			got := isNonRetryableErrorCode(tt.code)
			if got != tt.want {
				t.Errorf("isNonRetryableErrorCode(%q) = %v, want %v", tt.code, got, tt.want)
			}
		})
	}
}

// TestIsContentModerationErrorCode 测试内容审核类错误码判断
func TestIsContentModerationErrorCode(t *testing.T) {
	tests := []struct {
		code string
		want bool
	}{
		// 内容审核相关 - 不应重试
		{"sensitive_words_detected", true},
		{"content_policy_violation", true},
		{"content_filter", true},
		{"content_blocked", true},
		{"moderation_blocked", true},
		// 大小写不敏感
		{"SENSITIVE_WORDS_DETECTED", true},
		{"Content_Policy_Violation", true},
		// 参数校验类 - 不属于内容审核
		{"invalid_request", false},
		{"invalid_request_error", false},
		{"bad_request", false},
		// 其他错误码
		{"server_error", false},
		{"rate_limit", false},
		{"authentication_error", false},
		{"", false},
	}

	for _, tt := range tests {
		name := tt.code
		if name == "" {
			name = "empty"
		}
		t.Run(name, func(t *testing.T) {
			got := isContentModerationErrorCode(tt.code)
			if got != tt.want {
				t.Errorf("isContentModerationErrorCode(%q) = %v, want %v", tt.code, got, tt.want)
			}
		})
	}
}

// TestShouldRetryWithNextKey_SensitiveWordsDetected 测试敏感词检测错误不应重试
// 这是修复的核心场景：500 + sensitive_words_detected 不应触发无限重试
func TestShouldRetryWithNextKey_SensitiveWordsDetected(t *testing.T) {
	// 模拟生产环境的敏感词检测错误
	body := []byte(`{"error":{"message":"sensitive words detected","type":"new_api_error","param":"","code":"sensitive_words_detected"}}`)

	tests := []struct {
		name         string
		statusCode   int
		fuzzyMode    bool
		wantFailover bool
		wantQuota    bool
	}{
		{
			name:         "500 with sensitive_words_detected - normal mode",
			statusCode:   500,
			fuzzyMode:    false,
			wantFailover: false, // 不应重试
			wantQuota:    false,
		},
		{
			name:         "500 with sensitive_words_detected - fuzzy mode",
			statusCode:   500,
			fuzzyMode:    true,
			wantFailover: false, // 即使在 fuzzy 模式下也不应重试
			wantQuota:    false,
		},
		{
			name:         "400 with sensitive_words_detected - normal mode",
			statusCode:   400,
			fuzzyMode:    false,
			wantFailover: false,
			wantQuota:    false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotFailover, gotQuota := ShouldRetryWithNextKey(tt.statusCode, body, tt.fuzzyMode, "Messages")
			if gotFailover != tt.wantFailover {
				t.Errorf("ShouldRetryWithNextKey(%d, sensitive_words_body, %v) failover = %v, want %v",
					tt.statusCode, tt.fuzzyMode, gotFailover, tt.wantFailover)
			}
			if gotQuota != tt.wantQuota {
				t.Errorf("ShouldRetryWithNextKey(%d, sensitive_words_body, %v) quota = %v, want %v",
					tt.statusCode, tt.fuzzyMode, gotQuota, tt.wantQuota)
			}
		})
	}
}

// TestIsModelRoutingError 测试模型路由错误识别（仅用于状态码归一化）
func TestIsModelRoutingError(t *testing.T) {
	tests := []struct {
		name string
		body string
		want bool
	}{
		{
			name: "model_not_found code",
			body: `{"error":{"code":"model_not_found","message":"No available channel for model gpt-5.4 under group codex (distributor)","type":"new_api_error"}}`,
			want: true,
		},
		{
			name: "no available channel message without code",
			body: `{"error":{"message":"No available channel for model gpt-5.4 under group codex","type":"new_api_error"}}`,
			want: true,
		},
		{
			name: "model_not_found code case insensitive",
			body: `{"error":{"code":"MODEL_NOT_FOUND","message":"some error"}}`,
			want: true,
		},
		{
			name: "generic model not found without no available channel",
			body: `{"error":{"message":"model not found: gpt-5.4","type":"error"}}`,
			want: false,
		},
		{
			name: "quota error not client config",
			body: `{"error":{"type":"new_api_error","message":"预扣费额度失败, 用户剩余额度: ¥0.053950"}}`,
			want: false,
		},
		{
			name: "auth error not client config",
			body: `{"error":{"type":"new_api_error","message":"该令牌已过期","code":""}}`,
			want: false,
		},
		{
			name: "invalid request not client config",
			body: `{"error":{"code":"invalid_request","message":"bad request"}}`,
			want: false,
		},
		{
			name: "invalid json",
			body: `not json`,
			want: false,
		},
		{
			name: "no error object",
			body: `{"status":"ok"}`,
			want: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := isModelRoutingError([]byte(tt.body))
			if got != tt.want {
				t.Errorf("isModelRoutingError() = %v, want %v", got, tt.want)
			}
		})
	}
}

// TestNormalizeUpstreamErrorStatus 测试状态码归一化
func TestNormalizeUpstreamErrorStatus(t *testing.T) {
	modelNotFoundBody := []byte(`{"error":{"code":"model_not_found","message":"No available channel for model gpt-5.4 under group codex","type":"new_api_error"}}`)

	tests := []struct {
		name       string
		status     int
		body       []byte
		wantStatus int
	}{
		{"503 model_not_found normalizes to 404", 503, modelNotFoundBody, 404},
		{"500 model_not_found normalizes to 404", 500, modelNotFoundBody, 404},
		{"502 model_not_found normalizes to 404", 502, modelNotFoundBody, 404},
		{"403 model_not_found stays 403", 403, modelNotFoundBody, 403},
		{"200 model_not_found stays 200", 200, modelNotFoundBody, 200},
		{"503 empty body stays 503", 503, []byte{}, 503},
		{"503 nil body stays 503", 503, nil, 503},
		{"503 quota error stays 503", 503, []byte(`{"error":{"message":"quota exceeded"}}`), 503},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := normalizeUpstreamErrorStatus(tt.status, tt.body)
			if got != tt.wantStatus {
				t.Errorf("normalizeUpstreamErrorStatus(%d, ...) = %d, want %d", tt.status, got, tt.wantStatus)
			}
		})
	}
}

func TestHandleAllFailedFuzzyMode_ModelNotFoundNormalizesTo404(t *testing.T) {
	gin.SetMode(gin.TestMode)
	modelNotFoundBody := []byte(`{"error":{"code":"model_not_found","message":"No available channel for model gpt-5.4 under group codex","type":"new_api_error"}}`)
	quotaBody := []byte(`{"error":{"message":"quota exceeded"}}`)

	tests := []struct {
		name       string
		handle     func(*gin.Context)
		wantStatus int
	}{
		{
			name: "all channels failed - model_not_found normalizes to 404",
			handle: func(c *gin.Context) {
				HandleAllChannelsFailed(c, true, &FailoverError{Status: http.StatusServiceUnavailable, Body: modelNotFoundBody}, nil, "Messages")
			},
			wantStatus: http.StatusNotFound,
		},
		{
			name: "all keys failed - model_not_found normalizes to 404",
			handle: func(c *gin.Context) {
				HandleAllKeysFailed(c, true, &FailoverError{Status: http.StatusServiceUnavailable, Body: modelNotFoundBody}, nil, "Messages")
			},
			wantStatus: http.StatusNotFound,
		},
		{
			name: "all channels failed - 429 quota stays generic 503 in fuzzy mode",
			handle: func(c *gin.Context) {
				HandleAllChannelsFailed(c, true, &FailoverError{Status: http.StatusTooManyRequests, Body: quotaBody}, nil, "Messages")
			},
			wantStatus: http.StatusServiceUnavailable,
		},
		{
			name: "all keys failed - 429 quota stays generic 503 in fuzzy mode",
			handle: func(c *gin.Context) {
				HandleAllKeysFailed(c, true, &FailoverError{Status: http.StatusTooManyRequests, Body: quotaBody}, nil, "Messages")
			},
			wantStatus: http.StatusServiceUnavailable,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			recorder := httptest.NewRecorder()
			ctx, _ := gin.CreateTestContext(recorder)

			tt.handle(ctx)

			if recorder.Code != tt.wantStatus {
				t.Fatalf("status = %d, want %d", recorder.Code, tt.wantStatus)
			}
			if recorder.Body.String() == "" {
				t.Fatal("response body is empty")
			}
		})
	}
}

// TestShouldRetryWithNextKey_ModelNotFound 测试 model_not_found 允许 failover
// 模拟生产环境真实响应：上游 new-api 对 model_not_found 返回 503
// model_not_found 应允许 failover（不同 channel/上游实例可能支持该模型）
func TestShouldRetryWithNextKey_ModelNotFound(t *testing.T) {
	// 用户实际遇到的生产环境响应体
	body := []byte(`{"error":{"code":"model_not_found","message":"No available channel for model gpt-5.4 under group codex (distributor) (request id: 20260506023117409510104ddJSBEMJ)","type":"new_api_error"}}`)

	tests := []struct {
		name         string
		statusCode   int
		fuzzyMode    bool
		wantFailover bool
		wantQuota    bool
	}{
		{
			name:         "503 model_not_found - normal mode allows failover",
			statusCode:   503,
			fuzzyMode:    false,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "503 model_not_found - fuzzy mode allows failover",
			statusCode:   503,
			fuzzyMode:    true,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "500 model_not_found - normal mode allows failover",
			statusCode:   500,
			fuzzyMode:    false,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "500 model_not_found - fuzzy mode allows failover",
			statusCode:   500,
			fuzzyMode:    true,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "403 model_not_found - normal mode allows failover",
			statusCode:   403,
			fuzzyMode:    false,
			wantFailover: true,
			wantQuota:    false,
		},
		{
			name:         "403 model_not_found - fuzzy mode allows failover",
			statusCode:   403,
			fuzzyMode:    true,
			wantFailover: true,
			wantQuota:    false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotFailover, gotQuota := ShouldRetryWithNextKey(tt.statusCode, body, tt.fuzzyMode, "Messages")
			if gotFailover != tt.wantFailover {
				t.Errorf("ShouldRetryWithNextKey(%d, model_not_found_body, %v) failover = %v, want %v",
					tt.statusCode, tt.fuzzyMode, gotFailover, tt.wantFailover)
			}
			if gotQuota != tt.wantQuota {
				t.Errorf("ShouldRetryWithNextKey(%d, model_not_found_body, %v) quota = %v, want %v",
					tt.statusCode, tt.fuzzyMode, gotQuota, tt.wantQuota)
			}
		})
	}
}

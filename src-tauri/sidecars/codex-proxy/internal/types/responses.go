package types

// ============== Responses API 类型定义 ==============

// ResponsesRequest Responses API 请求
type ResponsesRequest struct {
	Model              string                   `json:"model"`
	Instructions       string                   `json:"instructions,omitempty"` // 系统指令（映射为 system message）
	Input              interface{}              `json:"input"`                  // string 或 []ResponsesItem
	PreviousResponseID string                   `json:"previous_response_id,omitempty"`
	Store              *bool                    `json:"store,omitempty"`               // 默认 true
	MaxTokens          int                      `json:"max_output_tokens,omitempty"`   // 最大 tokens
	Temperature        float64                  `json:"temperature,omitempty"`         // 温度参数
	TopP               float64                  `json:"top_p,omitempty"`               // top_p 参数
	FrequencyPenalty   float64                  `json:"frequency_penalty,omitempty"`   // 频率惩罚
	PresencePenalty    float64                  `json:"presence_penalty,omitempty"`    // 存在惩罚
	Stream             bool                     `json:"stream,omitempty"`              // 是否流式输出
	Stop               interface{}              `json:"stop,omitempty"`                // 停止序列 (string 或 []string)
	User               string                   `json:"user,omitempty"`                // 用户标识
	StreamOptions      interface{}              `json:"stream_options,omitempty"`      // 流式选项
	Tools              []map[string]interface{} `json:"-"`                             // function tools
	RawTools           []interface{}            `json:"tools,omitempty"`               // 原始工具定义，支持字符串简写
	ToolChoice         interface{}              `json:"tool_choice,omitempty"`         // string 或 object
	ParallelToolCalls  *bool                    `json:"parallel_tool_calls,omitempty"` // 是否允许并行工具调用

	// TransformerMetadata 转换器元数据（仅内存使用，不序列化）
	// 用于在单次请求的转换流程中保留原始格式信息，如 system 数组格式等
	// 注意：此字段不会通过 JSON 序列化保留，仅在同一请求处理链中有效
	TransformerMetadata map[string]interface{} `json:"-"`
}

// ResponsesItem Responses API 消息项
type ResponsesItem struct {
	ID        string      `json:"id,omitempty"`
	Type      string      `json:"type"`           // message, text, function_call, function_call_output（tool_call/tool_result 仅兼容 legacy Claude 输入）
	Role      string      `json:"role,omitempty"` // user, assistant (用于 type=message)
	Status    string      `json:"status,omitempty"`
	Content   interface{} `json:"content,omitempty"` // string 或 []ContentBlock
	Summary   interface{} `json:"summary,omitempty"`
	ToolUse   *ToolUse    `json:"tool_use,omitempty"`
	CallID    string      `json:"call_id,omitempty"`
	Name      string      `json:"name,omitempty"`
	Namespace string      `json:"namespace,omitempty"`
	Input     string      `json:"input,omitempty"`
	Arguments string      `json:"arguments,omitempty"`
	Output    interface{} `json:"output,omitempty"`
}

// ContentBlock 内容块（用于嵌套 content 数组）
type ContentBlock struct {
	Type string `json:"type"` // input_text, output_text
	Text string `json:"text"`
}

// ToolUse 工具使用定义
type ToolUse struct {
	ID    string      `json:"id"`
	Name  string      `json:"name"`
	Input interface{} `json:"input"`
}

// ResponsesResponse Responses API 响应
type ResponsesResponse struct {
	ID         string          `json:"id"`
	Model      string          `json:"model"`
	Output     []ResponsesItem `json:"output"`
	Status     string          `json:"status"` // completed, failed
	PreviousID string          `json:"previous_id,omitempty"`
	Usage      ResponsesUsage  `json:"usage"`
	Created    int64           `json:"created,omitempty"`
}

// ResponsesUsage Responses API 使用统计
// 完整支持 OpenAI Responses API 和 Claude API 的详细 usage 字段
// 参考 claude-code-hub 实现，支持缓存 TTL 细分 (5m/1h)
type ResponsesUsage struct {
	InputTokens         int                  `json:"input_tokens"`
	InputTokensDetails  *InputTokensDetails  `json:"input_tokens_details,omitempty"`
	OutputTokens        int                  `json:"output_tokens"`
	OutputTokensDetails *OutputTokensDetails `json:"output_tokens_details,omitempty"`
	TotalTokens         int                  `json:"total_tokens"`

	// Claude 扩展字段（缓存创建统计，用于精确计费）
	CacheCreationInputTokens   int    `json:"cache_creation_input_tokens,omitempty"`
	CacheCreation5mInputTokens int    `json:"cache_creation_5m_input_tokens,omitempty"` // 5分钟 TTL
	CacheCreation1hInputTokens int    `json:"cache_creation_1h_input_tokens,omitempty"` // 1小时 TTL
	CacheReadInputTokens       int    `json:"cache_read_input_tokens,omitempty"`
	CacheTTL                   string `json:"cache_ttl,omitempty"` // "5m" | "1h" | "mixed"
}

// InputTokensDetails 输入 Token 详细统计
type InputTokensDetails struct {
	CachedTokens int `json:"cached_tokens"`
}

// OutputTokensDetails 输出 Token 详细统计
type OutputTokensDetails struct {
	ReasoningTokens int `json:"reasoning_tokens"`
}

// ResponsesStreamEvent Responses API 流式事件
type ResponsesStreamEvent struct {
	ID         string          `json:"id,omitempty"`
	Model      string          `json:"model,omitempty"`
	Output     []ResponsesItem `json:"output,omitempty"`
	Status     string          `json:"status,omitempty"`
	PreviousID string          `json:"previous_id,omitempty"`
	Usage      *ResponsesUsage `json:"usage,omitempty"`
	Type       string          `json:"type,omitempty"` // delta, done
	Delta      *ResponsesDelta `json:"delta,omitempty"`
}

// ResponsesDelta 流式增量数据
type ResponsesDelta struct {
	Type    string      `json:"type,omitempty"`
	Content interface{} `json:"content,omitempty"`
}

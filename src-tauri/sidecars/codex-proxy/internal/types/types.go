package types

// ClaudeRequest Claude 请求结构
type ClaudeRequest struct {
	Model             string                 `json:"model"`
	Messages          []ClaudeMessage        `json:"messages"`
	System            interface{}            `json:"system,omitempty"` // string 或 content 数组
	MaxTokens         int                    `json:"max_tokens,omitempty"`
	Temperature       float64                `json:"temperature,omitempty"`
	TopP              float64                `json:"top_p,omitempty"`
	Stream            bool                   `json:"stream,omitempty"`
	Tools             []ClaudeTool           `json:"tools,omitempty"`
	ToolChoice        interface{}            `json:"tool_choice,omitempty"`
	ParallelToolCalls *bool                  `json:"parallel_tool_calls,omitempty"`
	Metadata          map[string]interface{} `json:"metadata,omitempty"` // Claude Code CLI 等客户端发送的元数据
}

// ClaudeMessage Claude 消息
type ClaudeMessage struct {
	Role    string      `json:"role"`
	Content interface{} `json:"content"` // string 或 content 数组
}

// CacheControl Anthropic 缓存控制
// 用于 Claude API 请求，会序列化到 JSON（仅在发送给 Anthropic 时有效）
type CacheControl struct {
	Type string `json:"type,omitempty"` // "ephemeral"
}

// ClaudeContent Claude 内容块
type ClaudeContent struct {
	Type         string        `json:"type"` // text, tool_use, tool_result
	Text         string        `json:"text,omitempty"`
	Thinking     string        `json:"thinking,omitempty"`
	Signature    string        `json:"signature,omitempty"`
	ID           string        `json:"id,omitempty"`
	Name         string        `json:"name,omitempty"`
	Input        interface{}   `json:"input,omitempty"`
	Content      interface{}   `json:"content,omitempty"` // tool_result 的内容字段
	ToolUseID    string        `json:"tool_use_id,omitempty"`
	CacheControl *CacheControl `json:"cache_control,omitempty"`
}

// ClaudeTool Claude 工具定义
type ClaudeTool struct {
	Name         string        `json:"name"`
	Description  string        `json:"description,omitempty"`
	InputSchema  interface{}   `json:"input_schema"`
	CacheControl *CacheControl `json:"cache_control,omitempty"`
}

// ClaudeResponse Claude 响应
type ClaudeResponse struct {
	ID         string          `json:"id"`
	Type       string          `json:"type"`
	Role       string          `json:"role"`
	Content    []ClaudeContent `json:"content"`
	StopReason string          `json:"stop_reason,omitempty"`
	Usage      *Usage          `json:"usage,omitempty"`
}

// OpenAIRequest OpenAI 请求结构
type OpenAIRequest struct {
	Model               string          `json:"model"`
	Messages            []OpenAIMessage `json:"messages"`
	MaxCompletionTokens int             `json:"max_completion_tokens,omitempty"`
	Temperature         float64         `json:"temperature,omitempty"`
	Stream              bool            `json:"stream,omitempty"`
	Tools               []OpenAITool    `json:"tools,omitempty"`
	ToolChoice          string          `json:"tool_choice,omitempty"`
}

// OpenAIMessage OpenAI 消息
type OpenAIMessage struct {
	Role             string           `json:"role"`
	Content          interface{}      `json:"content"` // string 或 null
	ReasoningContent string           `json:"reasoning_content,omitempty"`
	ToolCalls        []OpenAIToolCall `json:"tool_calls,omitempty"`
	ToolCallID       string           `json:"tool_call_id,omitempty"`
}

// OpenAIToolCall OpenAI 工具调用
type OpenAIToolCall struct {
	ID       string                 `json:"id"`
	Type     string                 `json:"type"`
	Function OpenAIToolCallFunction `json:"function"`
}

// OpenAIToolCallFunction OpenAI 工具调用函数
type OpenAIToolCallFunction struct {
	Name      string `json:"name"`
	Arguments string `json:"arguments"`
}

// OpenAITool OpenAI 工具定义
type OpenAITool struct {
	Type     string             `json:"type"`
	Function OpenAIToolFunction `json:"function"`
}

// OpenAIToolFunction OpenAI 工具函数
type OpenAIToolFunction struct {
	Name        string      `json:"name"`
	Description string      `json:"description,omitempty"`
	Parameters  interface{} `json:"parameters"`
}

// OpenAIResponse OpenAI 响应
type OpenAIResponse struct {
	ID      string         `json:"id"`
	Choices []OpenAIChoice `json:"choices"`
	Usage   *Usage         `json:"usage,omitempty"`
}

// OpenAIChoice OpenAI 选择
type OpenAIChoice struct {
	Message      OpenAIMessage `json:"message"`
	FinishReason string        `json:"finish_reason,omitempty"`
}

// Usage 使用情况统计
// 完整支持 Claude API 的详细 usage 字段，包括缓存 TTL 细分
type Usage struct {
	InputTokens              int `json:"input_tokens,omitempty"`
	OutputTokens             int `json:"output_tokens,omitempty"`
	CacheCreationInputTokens int `json:"cache_creation_input_tokens,omitempty"`
	CacheReadInputTokens     int `json:"cache_read_input_tokens,omitempty"`
	// PromptTokensTotal 仅供内部统计使用，用于保留上游返回的总 prompt tokens 口径。
	// 例如 Responses/OpenAI 风格的 input_tokens 可能已包含 cached tokens，metrics 层会据此归一化未命中输入。
	PromptTokensTotal int `json:"-"`
	// 缓存 TTL 细分（参考 claude-code-hub）
	CacheCreation5mInputTokens int    `json:"cache_creation_5m_input_tokens,omitempty"` // 5分钟 TTL
	CacheCreation1hInputTokens int    `json:"cache_creation_1h_input_tokens,omitempty"` // 1小时 TTL
	CacheTTL                   string `json:"cache_ttl,omitempty"`                      // "5m" | "1h" | "mixed"
	// OpenAI 兼容字段
	PromptTokens     int `json:"prompt_tokens,omitempty"`
	CompletionTokens int `json:"completion_tokens,omitempty"`
}

// ProviderRequest 提供商请求（通用）
type ProviderRequest struct {
	URL     string
	Method  string
	Headers map[string]string
	Body    interface{}
}

// ProviderResponse 提供商响应（通用）
type ProviderResponse struct {
	StatusCode int
	Headers    map[string][]string
	Body       []byte
	Stream     bool
}

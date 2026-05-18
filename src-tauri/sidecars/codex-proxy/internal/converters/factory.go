package converters

// ConverterFactory 转换器工厂
// 根据上游服务类型返回对应的转换器实例

// NewConverter 创建转换器实例
// serviceType: "openai", "claude", "gemini", "responses"
func NewConverter(serviceType string) ResponsesConverter {
	switch serviceType {
	case "openai":
		return &OpenAIChatConverter{}
	case "claude":
		return &ClaudeConverter{}
	case "gemini":
		return &GeminiResponsesConverter{}
	case "responses":
		return &ResponsesPassthroughConverter{}
	default:
		// 默认使用 OpenAI Chat 转换器
		return &OpenAIChatConverter{}
	}
}

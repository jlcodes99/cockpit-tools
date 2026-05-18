package handlers

import (
	"fmt"
	"strings"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
)

// ============== 复合协议类型定义 ==============

const compositeProtocolSep = "->"

// CapabilityBaseProtocol 基础协议枚举（messages / chat / responses / gemini）
type CapabilityBaseProtocol string

const (
	CapabilityProtocolMessages  CapabilityBaseProtocol = "messages"
	CapabilityProtocolChat      CapabilityBaseProtocol = "chat"
	CapabilityProtocolResponses CapabilityBaseProtocol = "responses"
	CapabilityProtocolGemini    CapabilityBaseProtocol = "gemini"
)

// allBaseProtocols 全部基础协议，用于 4×4 矩阵遍历
var allBaseProtocols = []CapabilityBaseProtocol{
	CapabilityProtocolMessages,
	CapabilityProtocolChat,
	CapabilityProtocolResponses,
	CapabilityProtocolGemini,
}

// normalizeServiceTypeToProtocol 将渠道 ServiceType 归一化为 CapabilityBaseProtocol。
// 返回 (protocol, ok)；ok=false 表示 serviceType 无法映射（如 images）。
func normalizeServiceTypeToProtocol(serviceType string) (CapabilityBaseProtocol, bool) {
	switch strings.ToLower(strings.TrimSpace(serviceType)) {
	case "claude", "messages":
		return CapabilityProtocolMessages, true
	case "openai", "openai-chat", "chat":
		return CapabilityProtocolChat, true
	case "responses", "codex":
		return CapabilityProtocolResponses, true
	case "gemini":
		return CapabilityProtocolGemini, true
	default:
		return "", false
	}
}

// buildCompositeProtocol 组装复合协议键，形如 "messages->responses"。
func buildCompositeProtocol(from, to CapabilityBaseProtocol) string {
	return string(from) + compositeProtocolSep + string(to)
}

// parseCompositeProtocol 解析复合协议键，返回 (from, to, ok)。
// 若 protocol 不含分隔符，ok=false。
func parseCompositeProtocol(protocol string) (CapabilityBaseProtocol, CapabilityBaseProtocol, bool) {
	idx := strings.Index(protocol, compositeProtocolSep)
	if idx < 0 {
		return "", "", false
	}
	from := CapabilityBaseProtocol(protocol[:idx])
	to := CapabilityBaseProtocol(protocol[idx+len(compositeProtocolSep):])
	if from == "" || to == "" {
		return "", "", false
	}
	return from, to, true
}

// isCompositeProtocol 判断 protocol 是否为复合协议（含 "->" 分隔符）。
func isCompositeProtocol(protocol string) bool {
	return strings.Contains(protocol, compositeProtocolSep)
}

// hasModelMapping 判断渠道是否配置了 ModelMapping。
func hasModelMapping(channel *config.UpstreamConfig) bool {
	if channel == nil {
		return false
	}
	return len(channel.ModelMapping) > 0
}

// expandCapabilityProtocolsForChannel 根据渠道 ModelMapping 扩展协议列表。
// 当 ModelMapping 非空时，在 protocols 前插入一条复合协议行 {channelKind}->{serviceType}。
// 复合协议行始终排在第一位；同协议也保留，用于验证实际 ModelMapping 后的用户使用路径。
func expandCapabilityProtocolsForChannel(channelKind string, channel *config.UpstreamConfig, protocols []string) []string {
	if !hasModelMapping(channel) {
		return protocols
	}

	from := CapabilityBaseProtocol(channelKind)
	to, ok := normalizeServiceTypeToProtocol(channel.ServiceType)
	if !ok {
		return protocols
	}

	composite := buildCompositeProtocol(from, to)

	// 检查是否已存在
	for _, p := range protocols {
		if p == composite {
			return protocols
		}
	}

	// 复合协议排在第一位
	expanded := make([]string, 0, len(protocols)+1)
	expanded = append(expanded, composite)
	expanded = append(expanded, protocols...)
	return expanded
}

// getProbeModelsForCapabilityProtocol 获取指定协议的探测模型列表。
// 普通协议：直接查 capabilityProbeModels 表。
// 复合协议：使用 from 方向的探测模型（复合协议行展示的是入口侧探测模型）。
func getProbeModelsForCapabilityProtocol(protocol string) ([]string, error) {
	if from, _, ok := parseCompositeProtocol(protocol); ok {
		return getCapabilityProbeModels(string(from))
	}
	return getCapabilityProbeModels(protocol)
}

// compositePathRegistry 4×4 复合协议请求构造器注册表。
// key = "from->to"，value = 构造上游请求的函数。
// 若 key 不在表中，视为 unsupported_composite_path。
type compositePathBuilder func(
	channel *config.UpstreamConfig,
	apiKey string,
	probeModel string,
) (reqURL string, reqBody []byte, targetProtocol string, err error)

var compositePathRegistry = map[string]compositePathBuilder{}

// registerCompositePath 注册一条复合协议路径。
func registerCompositePath(from, to CapabilityBaseProtocol, builder compositePathBuilder) {
	key := buildCompositeProtocol(from, to)
	compositePathRegistry[key] = builder
}

// getCompositePathBuilder 获取复合协议请求构造器。
func getCompositePathBuilder(from, to CapabilityBaseProtocol) (compositePathBuilder, bool) {
	key := buildCompositeProtocol(from, to)
	builder, ok := compositePathRegistry[key]
	return builder, ok
}

// unsupportedCompositePathErr 返回复合协议路径不支持的错误。
func unsupportedCompositePathErr(from, to CapabilityBaseProtocol) error {
	return fmt.Errorf("unsupported_composite_path: %s->%s", from, to)
}

func init() {
	// ============== messages -> messages（原生协议重定向测试）==============
	registerCompositePath(CapabilityProtocolMessages, CapabilityProtocolMessages,
		func(channel *config.UpstreamConfig, apiKey, probeModel string) (string, []byte, string, error) {
			// 构造 messages 入口最小请求体
			messagesBody := buildMessagesProbeBody(probeModel)
			// 使用 ClaudeProvider 将 messages 请求转换为 messages 上游请求（会应用 ModelMapping）
			return buildCompositeRequestViaProvider("messages", channel, apiKey, messagesBody, "/v1/messages")
		},
	)

	// ============== messages -> responses（首要路径）==============
	registerCompositePath(CapabilityProtocolMessages, CapabilityProtocolResponses,
		func(channel *config.UpstreamConfig, apiKey, probeModel string) (string, []byte, string, error) {
			// 构造 messages 入口最小请求体
			messagesBody := buildMessagesProbeBody(probeModel)
			// 使用 ResponsesProvider 将 messages 请求转换为 responses 上游请求
			return buildCompositeRequestViaProvider("responses", channel, apiKey, messagesBody, "/v1/messages")
		},
	)

	// ============== messages -> chat ==============
	registerCompositePath(CapabilityProtocolMessages, CapabilityProtocolChat,
		func(channel *config.UpstreamConfig, apiKey, probeModel string) (string, []byte, string, error) {
			messagesBody := buildMessagesProbeBody(probeModel)
			return buildCompositeRequestViaProvider("chat", channel, apiKey, messagesBody, "/v1/messages")
		},
	)

	// ============== chat -> messages ==============
	registerCompositePath(CapabilityProtocolChat, CapabilityProtocolMessages,
		func(channel *config.UpstreamConfig, apiKey, probeModel string) (string, []byte, string, error) {
			chatBody := buildChatProbeBody(probeModel)
			return buildCompositeRequestViaProvider("messages", channel, apiKey, chatBody, "/v1/chat/completions")
		},
	)

	// ============== chat -> responses ==============
	registerCompositePath(CapabilityProtocolChat, CapabilityProtocolResponses,
		func(channel *config.UpstreamConfig, apiKey, probeModel string) (string, []byte, string, error) {
			chatBody := buildChatProbeBody(probeModel)
			return buildCompositeRequestViaProvider("responses", channel, apiKey, chatBody, "/v1/chat/completions")
		},
	)

	// ============== responses -> messages ==============
	registerCompositePath(CapabilityProtocolResponses, CapabilityProtocolMessages,
		func(channel *config.UpstreamConfig, apiKey, probeModel string) (string, []byte, string, error) {
			responsesBody := buildResponsesProbeBody(probeModel)
			return buildCompositeRequestViaProvider("messages", channel, apiKey, responsesBody, "/v1/responses")
		},
	)

	// ============== responses -> chat ==============
	registerCompositePath(CapabilityProtocolResponses, CapabilityProtocolChat,
		func(channel *config.UpstreamConfig, apiKey, probeModel string) (string, []byte, string, error) {
			responsesBody := buildResponsesProbeBody(probeModel)
			return buildCompositeRequestViaProvider("chat", channel, apiKey, responsesBody, "/v1/responses")
		},
	)

	// ============== chat -> chat（原生协议重定向测试）==============
	registerCompositePath(CapabilityProtocolChat, CapabilityProtocolChat,
		func(channel *config.UpstreamConfig, apiKey, probeModel string) (string, []byte, string, error) {
			chatBody := buildChatProbeBody(probeModel)
			return buildCompositeRequestViaProvider("chat", channel, apiKey, chatBody, "/v1/chat/completions")
		},
	)

	// ============== responses -> responses（原生协议重定向测试）==============
	registerCompositePath(CapabilityProtocolResponses, CapabilityProtocolResponses,
		func(channel *config.UpstreamConfig, apiKey, probeModel string) (string, []byte, string, error) {
			responsesBody := buildResponsesProbeBody(probeModel)
			return buildCompositeRequestViaProvider("responses", channel, apiKey, responsesBody, "/v1/responses")
		},
	)

	// ============== gemini -> gemini（原生协议重定向测试）==============
	registerCompositePath(CapabilityProtocolGemini, CapabilityProtocolGemini,
		func(channel *config.UpstreamConfig, apiKey, probeModel string) (string, []byte, string, error) {
			geminiBody := buildGeminiProbeBody(probeModel)
			return buildCompositeRequestViaProvider("gemini", channel, apiKey, geminiBody, "/v1beta/models/probe:streamGenerateContent?alt=sse")
		},
	)
}

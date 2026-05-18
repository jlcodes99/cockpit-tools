package converters

// NormalizeNonstandardChatRolesInRequest 将 OpenAI Chat 请求中非标准 role 改写为 user。
// 标准 role 保持为兼容面最广的 system/user/assistant/tool。
func NormalizeNonstandardChatRolesInRequest(reqMap map[string]interface{}) {
	switch messages := reqMap["messages"].(type) {
	case []interface{}:
		for _, msg := range messages {
			m, ok := msg.(map[string]interface{})
			if !ok {
				continue
			}
			normalizeChatMessageRole(m)
		}
	case []map[string]interface{}:
		for _, msg := range messages {
			normalizeChatMessageRole(msg)
		}
	}
}

func normalizeChatMessageRole(msg map[string]interface{}) {
	role, ok := msg["role"].(string)
	if !ok {
		msg["role"] = "user"
		return
	}
	if isStandardChatRole(role) {
		return
	}
	// 非标准 role 若携带 tool_call_id，视为 tool 响应（保留 OpenAI tool_calls 响应链完整性）
	if _, hasID := msg["tool_call_id"]; hasID {
		msg["role"] = "tool"
		return
	}
	msg["role"] = "user"
}

func isStandardChatRole(role string) bool {
	switch role {
	case "system", "user", "assistant", "tool":
		return true
	default:
		return false
	}
}

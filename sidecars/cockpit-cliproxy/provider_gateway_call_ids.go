package main

import cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor"

// normalizeProviderGatewayCallIDs 给历史工具调用补齐缺失的 call_id。
//
// Codex 客户端直连 sidecar，不会经过宿主 Rust 的补 ID。DeepSeek 这类严格
// Responses 上游按字段反序列化整个 input，缺 call_id 会整包 422
// （missing field `call_id`）。这里复用 Codex 执行器同一套配对规则：能配对的
// 调用和输出共用合成 ID，对不上的匿名输出丢掉，已有 ID 不改。
func normalizeProviderGatewayCallIDs(body []byte) []byte {
	return cliproxyexecutor.NormalizeCodexCallIDs(body)
}

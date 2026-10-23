//go:build continuity_integration

package main

import (
	"strings"
	"testing"
)

func TestRealContinuityWorkerLegacyToolsAgentCipherAndLargeHistoryAcrossRestartAndAccounts(t *testing.T) {
	legacy := []any{
		map[string]any{"type": "reasoning", "summary": []any{}, "encrypted_content": "imported-reasoning"},
		map[string]any{"type": "local_shell_call", "id": "legacy-shell", "call_id": "shell-call", "status": "completed", "action": map[string]any{"type": "exec", "command": []any{"echo", "ok"}}},
		map[string]any{"type": "function_call_output", "call_id": "shell-call", "output": "ok"},
		map[string]any{"type": "function_call", "namespace": "collaboration", "name": "spawn_agent", "call_id": "error-call", "arguments": `{"message":`},
		map[string]any{"type": "function_call_output", "call_id": "error-call", "output": "Error parsing function arguments: EOF"},
		map[string]any{"type": "agent_message", "id": "amsg-old", "author": "/root/child", "recipient": "/root", "content": []any{map[string]any{"type": "encrypted_content", "encrypted_content": "imported-agent-cipher"}}},
		map[string]any{"type": "agent_message", "id": "amsg-plain", "author": "/root", "recipient": "/root/child", "content": []any{map[string]any{"type": "input_text", "text": "known plaintext task"}}},
	}
	// Exceed the hot checkpoint cache and enable the real official-client
	// optimizer. Pure-ID continuation must still preserve every legacy item.
	runRealContinuityWorkerHistory(t, strings.Repeat("x", 9*1024*1024), true, legacy, true, true)
}

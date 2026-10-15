package helps

import (
	"context"
	"strings"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/tidwall/gjson"
)

// ObserveCodexCompletionUsage distinguishes complete output from terminal
// partial usage. A success record alone does not prove generation completed.
func ObserveCodexCompletionUsage(ctx context.Context, payload []byte) {
	event := gjson.GetBytes(payload, "type").String()
	status := strings.TrimSpace(gjson.GetBytes(payload, "response.status").String())
	input := gjson.GetBytes(payload, "response.usage.input_tokens")
	output := gjson.GetBytes(payload, "response.usage.output_tokens")
	complete := (event == "response.completed" || event == "response.done") &&
		(status == "" || status == "completed") &&
		input.Type == gjson.Number && input.Int() >= 0 && input.Float() == float64(input.Int()) &&
		output.Type == gjson.Number && output.Int() >= 0 && output.Float() == float64(output.Int()) &&
		gjson.GetBytes(payload, "response.error").Type == gjson.Null
	cliproxyexecutor.ObserveUpstreamCompletion(ctx, complete)
}

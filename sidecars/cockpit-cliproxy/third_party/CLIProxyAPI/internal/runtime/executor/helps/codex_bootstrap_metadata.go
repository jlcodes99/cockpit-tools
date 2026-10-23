package helps

import "github.com/tidwall/gjson"

// CodexHandshakeMetadataEvent holds only recognized frames with no evidence of
// execution. Echoed tool schemas and response.text are configuration, not output.
func CodexHandshakeMetadataEvent(payload []byte) bool {
	switch gjson.GetBytes(payload, "type").String() {
	case "response.created", "response.in_progress", "response.metadata", "codex.rate_limits", "codex.response.metadata":
		return CodexEventHasNoExecution(payload)
	default:
		return false
	}
}

// CodexEventHasNoExecution also fences terminal errors which carry generated
// output themselves. Unknown/malformed output must not authorize replay.
func CodexEventHasNoExecution(payload []byte) bool {
	if !gjson.ValidBytes(payload) {
		return false
	}
	event := gjson.ParseBytes(payload)
	return codexMetadataObject(event, false) &&
		codexMetadataObject(event.Get("response"), true) &&
		codexMetadataObject(event.Get("metadata"), false) &&
		codexMetadataObject(event.Get("response.metadata"), false) &&
		codexMetadataObject(event.Get("body"), false) &&
		codexMetadataObject(event.Get("body.response"), true)
}

func codexMetadataObject(node gjson.Result, response bool) bool {
	if !node.Exists() || node.Type == gjson.Null {
		return true
	}
	if !node.IsObject() {
		return false
	}
	valid := true
	node.ForEach(func(key, value gjson.Result) bool {
		if value.Type == gjson.Null {
			return true
		}
		switch key.String() {
		case "output":
			valid = value.IsArray() && len(value.Array()) == 0
		case "usage":
			valid = codexNoOutputUsage(value)
		case "output_tokens", "completion_tokens", "reasoning_tokens":
			valid = codexZeroTokenCount(value)
		case "text":
			valid = response && codexTextConfiguration(value)
		case "reasoning":
			valid = response && codexReasoningConfiguration(value)
		case "delta", "item", "content", "arguments", "tool_calls", "function_call", "function_call_output", "output_text", "audio", "image", "partial_json", "reasoning_text", "encrypted_content", "summary":
			valid = false
		}
		return valid
	})
	return valid
}

func codexZeroTokenCount(value gjson.Result) bool {
	return !value.Exists() || value.Type == gjson.Null || value.Type == gjson.Number && value.Raw == "0"
}

func codexNoOutputUsage(value gjson.Result) bool {
	if !value.IsObject() {
		return false
	}
	for _, path := range []string{"output_tokens", "completion_tokens", "output_tokens_details.reasoning_tokens", "completion_tokens_details.reasoning_tokens"} {
		if !codexZeroTokenCount(value.Get(path)) {
			return false
		}
	}
	return true
}

func codexTextConfiguration(node gjson.Result) bool {
	if !node.IsObject() {
		return false
	}
	valid := true
	node.ForEach(func(key, value gjson.Result) bool {
		switch key.String() {
		case "verbosity":
			valid = value.Type == gjson.Null || value.Type == gjson.String && (value.Str == "low" || value.Str == "medium" || value.Str == "high")
		case "format":
			valid = value.Type == gjson.Null || codexTextFormat(value)
		default:
			valid = false
		}
		return valid
	})
	return valid
}

func codexTextFormat(node gjson.Result) bool {
	if !node.IsObject() {
		return false
	}
	if node.Raw == "{}" {
		return true
	}
	kind := node.Get("type").String()
	if kind != "text" && kind != "json_object" && kind != "json_schema" {
		return false
	}
	valid := true
	node.ForEach(func(key, _ gjson.Result) bool {
		switch key.String() {
		case "type", "name", "schema", "strict", "description":
		default:
			valid = false
		}
		return valid
	})
	return valid
}

func codexReasoningConfiguration(node gjson.Result) bool {
	if !node.IsObject() {
		return false
	}
	valid := true
	node.ForEach(func(key, value gjson.Result) bool {
		switch key.String() {
		case "effort":
			valid = value.Type == gjson.Null || value.Type == gjson.String && (value.Str == "none" || value.Str == "minimal" || value.Str == "low" || value.Str == "medium" || value.Str == "high" || value.Str == "xhigh")
		case "summary", "generate_summary":
			valid = value.Type == gjson.Null || value.Type == gjson.String && (value.Str == "auto" || value.Str == "concise" || value.Str == "detailed")
		default:
			valid = false
		}
		return valid
	})
	return valid
}

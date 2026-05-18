package converters

const omittedImagePlaceholder = "[Image omitted: current upstream channel does not support image input.]"

// OmitOpenAIChatImageContent removes OpenAI-compatible image blocks while
// preserving text blocks, so text-only providers can still accept history that
// contains screenshots or other image attachments.
func OmitOpenAIChatImageContent(reqMap map[string]interface{}) bool {
	changed := false
	switch messages := reqMap["messages"].(type) {
	case []interface{}:
		for _, rawMessage := range messages {
			message, ok := rawMessage.(map[string]interface{})
			if ok && omitOpenAIChatMessageImageContent(message) {
				changed = true
			}
		}
	case []map[string]interface{}:
		for _, message := range messages {
			if omitOpenAIChatMessageImageContent(message) {
				changed = true
			}
		}
	default:
		return false
	}
	return changed
}

func omitOpenAIChatMessageImageContent(message map[string]interface{}) bool {
	switch content := message["content"].(type) {
	case []interface{}:
		sanitized, didChange := omitOpenAIChatImageParts(content)
		if didChange {
			message["content"] = sanitized
		}
		return didChange
	case []map[string]interface{}:
		sanitized, didChange := omitOpenAIChatImageMapParts(content)
		if didChange {
			message["content"] = sanitized
		}
		return didChange
	default:
		return false
	}
}

func omitOpenAIChatImageParts(content []interface{}) ([]interface{}, bool) {
	sanitized := make([]interface{}, 0, len(content)+1)
	omitted := false
	for _, rawPart := range content {
		part, ok := rawPart.(map[string]interface{})
		if !ok {
			sanitized = append(sanitized, rawPart)
			continue
		}

		partType, _ := part["type"].(string)
		switch partType {
		case "image", "image_url", "input_image":
			omitted = true
			continue
		default:
			sanitized = append(sanitized, rawPart)
		}
	}

	if omitted {
		sanitized = append(sanitized, map[string]interface{}{
			"type": "text",
			"text": omittedImagePlaceholder,
		})
	}
	if len(sanitized) == 0 {
		sanitized = append(sanitized, map[string]interface{}{
			"type": "text",
			"text": omittedImagePlaceholder,
		})
	}

	return sanitized, omitted
}

func omitOpenAIChatImageMapParts(content []map[string]interface{}) ([]map[string]interface{}, bool) {
	sanitized := make([]map[string]interface{}, 0, len(content)+1)
	omitted := false
	for _, part := range content {
		partType, _ := part["type"].(string)
		switch partType {
		case "image", "image_url", "input_image":
			omitted = true
			continue
		default:
			sanitized = append(sanitized, part)
		}
	}

	if omitted {
		sanitized = append(sanitized, map[string]interface{}{
			"type": "text",
			"text": omittedImagePlaceholder,
		})
	}
	if len(sanitized) == 0 {
		sanitized = append(sanitized, map[string]interface{}{
			"type": "text",
			"text": omittedImagePlaceholder,
		})
	}

	return sanitized, omitted
}

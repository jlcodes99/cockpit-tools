package common

import (
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
	"github.com/tidwall/gjson"
)

const visionDetectedContextKey = "codex_proxy_has_image_content"

// HasImageContent 检测请求体是否包含图片内容（覆盖 Claude/OpenAI/Responses/Gemini 四种协议格式）。
// 结果缓存在 gin.Context 中，failover 重试时不重复解析。
func HasImageContent(c *gin.Context, bodyBytes []byte) bool {
	if cached, exists := c.Get(visionDetectedContextKey); exists {
		return cached.(bool)
	}
	detected := detectImageInBody(bodyBytes)
	c.Set(visionDetectedContextKey, detected)
	return detected
}

func detectImageInBody(body []byte) bool {
	if len(body) == 0 {
		return false
	}

	// Claude Messages / OpenAI Chat: messages[*].content[*].type == "image" | "image_url"
	messages := gjson.GetBytes(body, "messages")
	if messages.Exists() && messages.IsArray() {
		for _, msg := range messages.Array() {
			content := msg.Get("content")
			if content.IsArray() {
				for _, block := range content.Array() {
					t := block.Get("type").String()
					if t == "image" || t == "image_url" {
						return true
					}
				}
			}
		}
	}

	// Responses API: input[*].type == "input_image" 或嵌套 content 中的 input_image
	input := gjson.GetBytes(body, "input")
	if input.Exists() && input.IsArray() {
		for _, item := range input.Array() {
			t := item.Get("type").String()
			if t == "input_image" {
				return true
			}
			// 嵌套 content 数组（如 input_message.content）
			itemContent := item.Get("content")
			if itemContent.IsArray() {
				for _, block := range itemContent.Array() {
					if block.Get("type").String() == "input_image" {
						return true
					}
				}
			}
		}
	}

	// Gemini: contents[*].parts[*].inlineData 或 fileData（含 image MIME）
	contents := gjson.GetBytes(body, "contents")
	if contents.Exists() && contents.IsArray() {
		for _, c := range contents.Array() {
			parts := c.Get("parts")
			if parts.IsArray() {
				for _, part := range parts.Array() {
					if part.Get("inlineData").Exists() || part.Get("fileData").Exists() {
						return true
					}
				}
			}
		}
	}

	return false
}

// isNoVisionModel 检查模型是否在渠道的 NoVisionModels 列表中（精确匹配）。
func isNoVisionModel(upstream *config.UpstreamConfig, model string) bool {
	for _, m := range upstream.NoVisionModels {
		if m == model {
			return true
		}
	}
	return false
}

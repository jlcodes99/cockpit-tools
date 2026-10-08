package executor

import (
	"context"
	"fmt"
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/signature"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/util"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// normalizeNonOfficialCodexReasoningItems 清洗第三方来源的 reasoning 项。
//
// 官方 Codex 后端要求 reasoning 项的 content 必须是空数组，而第三方提供商
// （DeepSeek 等）在历史里回放的是带可见 reasoning_text 的 reasoning 项。
// 同一会话在第三方与官方账号之间来回切换后，官方后端会直接拒绝整段请求：
//
//	[ArrayParam] [input[<i>].content] [array_above_max_length]
//	Invalid 'input[<i>].content': array too long. Expected an array with
//	maximum length 0, but got an array with length 1 instead.
//
// 这里把这类项的 content 清空；清空后既没有 summary 也没有合法加密内容的项
// 只携带第三方推理文本，官方后端无法复用，直接丢弃，避免留下空壳 reasoning 项。
func normalizeNonOfficialCodexReasoningItems(ctx context.Context, provider string, body []byte) []byte {
	inputResult := util.GetGJSONBytesNoCopy(body, "input")
	if !inputResult.Exists() || !inputResult.IsArray() {
		return body
	}
	provider = strings.TrimSpace(provider)
	if provider == "" {
		provider = "codex upstream"
	}

	items := inputResult.Array()

	// rebuilt 只在真的需要改动时初始化，未命中的请求不产生额外分配。
	var rebuilt []byte
	itemsWritten := 0
	keep := func(raw string) {
		if rebuilt == nil {
			return
		}
		if itemsWritten > 0 {
			rebuilt = append(rebuilt, ',')
		}
		rebuilt = append(rebuilt, raw...)
		itemsWritten++
	}
	startRebuild := func(index int) {
		if rebuilt != nil {
			return
		}
		rebuilt = make([]byte, 0, len(inputResult.Raw))
		rebuilt = append(rebuilt, '[')
		for i := range index {
			keep(items[i].Raw)
		}
	}

	for index, item := range items {
		if strings.TrimSpace(item.Get("type").String()) != "reasoning" {
			keep(item.Raw)
			continue
		}
		content := item.Get("content")
		if !content.IsArray() || len(content.Array()) == 0 {
			keep(item.Raw)
			continue
		}

		itemID := strings.TrimSpace(item.Get("id").String())
		if itemID == "" {
			itemID = fmt.Sprintf("input[%d]", index)
		}
		cleared, err := sjson.SetRawBytes([]byte(item.Raw), "content", []byte("[]"))
		if err != nil {
			helps.LogWithRequestID(ctx).Debugf("%s: failed to clear reasoning content at input[%d]: %v", provider, index, err)
			keep(item.Raw)
			continue
		}

		startRebuild(index)
		if !codexReasoningItemHasReplayState(gjson.ParseBytes(cleared)) {
			helps.LogWithRequestID(ctx).Debugf("%s: dropped non-official reasoning item at input[%d] item_id=%q reason=only visible reasoning content", provider, index, itemID)
			continue
		}
		keep(string(cleared))
		helps.LogWithRequestID(ctx).Debugf("%s: cleared non-official reasoning content at input[%d] item_id=%q", provider, index, itemID)
	}
	if rebuilt == nil {
		return body
	}
	rebuilt = append(rebuilt, ']')

	updated, err := sjson.SetRawBytes(body, "input", rebuilt)
	if err != nil {
		helps.LogWithRequestID(ctx).Debugf("%s: failed to rebuild input array while clearing reasoning content: %v", provider, err)
		return body
	}
	return updated
}

// codexReasoningItemHasReplayState 判断 reasoning 项在官方后端是否还有可复用状态：
// 非空 summary，或合法的 GPT 加密推理内容。
func codexReasoningItemHasReplayState(item gjson.Result) bool {
	if len(item.Get("summary").Array()) > 0 {
		return true
	}
	encryptedContent := item.Get("encrypted_content")
	if encryptedContent.Type != gjson.String {
		return false
	}
	return signature.IsValidGPTReasoningSignature(encryptedContent.String())
}

package executor

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/auth/codebuddy"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
	log "github.com/sirupsen/logrus"
)

// codebuddyVisionAction describes how the vision-proxy layer handles a request.
type codebuddyVisionAction int

const (
	// codebuddyVisionPassThrough leaves the request unchanged.
	codebuddyVisionPassThrough codebuddyVisionAction = iota
	// codebuddyVisionRoute swaps the request model to the configured vision model.
	codebuddyVisionRoute
	// codebuddyVisionPreprocess describes images first, then continues with the
	// original model.
	codebuddyVisionPreprocess
)

// defaultCodebuddyVisionPrompt is the system prompt sent to the vision model in
// preprocess mode when no override is configured.
const defaultCodebuddyVisionPrompt = "请仔细观察图片，用中文详细、准确地描述图片内容。"

// codebuddyOmittedImageText replaces image parts when preprocess fails and the
// request degrades to omitting images.
const codebuddyOmittedImageText = "[图片因视觉代理失败被省略]"

// codebuddyHistoricalImageText replaces historical image parts in agentic mode
// so a later turn does not re-send stale images to a text-only model that
// rejects image input.
const codebuddyHistoricalImageText = "[历史图片]"

func (a codebuddyVisionAction) String() string {
	switch a {
	case codebuddyVisionRoute:
		return "routing"
	case codebuddyVisionPreprocess:
		return "preprocess"
	default:
		return "pass-through"
	}
}

// lastCodebuddyUserMessageIndex returns the index of the last role=="user"
// message, or -1 if there is none. Detecting images only in the last user
// message isolates the "current request" from historical turns, so a text-only
// follow-up in the same session is not misclassified by a previous image.
func lastCodebuddyUserMessageIndex(messages []gjson.Result) int {
	for i := len(messages) - 1; i >= 0; i-- {
		if messages[i].Get("role").String() == "user" {
			return i
		}
	}
	return -1
}

// codebuddyChatHasImageInput reports whether the OpenAI-style chat body carries
// at least one image part (image_url or input_image) in the current turn — the
// last user message and any subsequent assistant/tool messages. This covers both
// direct image attachment (user message) and Read-tool image results (tool
// message), while excluding earlier historical messages so a text-only follow-up
// in the same session is not misclassified by a previous image.
func codebuddyChatHasImageInput(body []byte) bool {
	messages := gjson.GetBytes(body, "messages")
	if !messages.IsArray() {
		return false
	}
	arr := messages.Array()
	lastUserIdx := lastCodebuddyUserMessageIndex(arr)
	if lastUserIdx < 0 {
		return false
	}
	for mi := lastUserIdx; mi < len(arr); mi++ {
		content := arr[mi].Get("content")
		if !content.IsArray() {
			continue
		}
		for _, part := range content.Array() {
			if isCodebuddyImagePartType(part.Get("type").String()) {
				return true
			}
		}
	}
	return false
}

// isCodebuddyImagePartType reports whether a content-part type carries image
// input (OpenAI image_url or Anthropic-style input_image).
func isCodebuddyImagePartType(typ string) bool {
	return typ == "image_url" || typ == "input_image"
}

// rewriteCodebuddyModel replaces the top-level model field with model.
func rewriteCodebuddyModel(body []byte, model string) []byte {
	out, err := sjson.SetBytes(body, "model", model)
	if err != nil {
		return body
	}
	return out
}

// replaceCodebuddyImagesWithText replaces every image part (image_url /
// input_image) with a text part carrying the provided description. Non-image
// parts are left untouched, and the replacement preserves the part's position
// within the message content array.
func replaceCodebuddyImagesWithText(body []byte, text string) []byte {
	messages := gjson.GetBytes(body, "messages")
	if !messages.IsArray() {
		return body
	}

	out := body
	for mi, msg := range messages.Array() {
		content := msg.Get("content")
		if !content.IsArray() {
			continue
		}
		for ci, part := range content.Array() {
			if !isCodebuddyImagePartType(part.Get("type").String()) {
				continue
			}
			path := fmt.Sprintf("messages.%d.content.%d", mi, ci)
			replacement, err := json.Marshal(map[string]string{"type": "text", "text": text})
			if err != nil {
				return body
			}
			out, err = sjson.SetRawBytes(out, path, replacement)
			if err != nil {
				// Never corrupt the request on a path error; keep the original.
				return body
			}
		}
	}
	return out
}

// codebuddyVisionPlan decides how the vision-proxy layer should handle the
// request. It is a pure function (no I/O) so routing decisions stay unit-testable.
func codebuddyVisionPlan(mode, visionModel, currentModel string, hasImage, currentSupportsImages bool) codebuddyVisionAction {
	if mode != config.CodebuddyVisionModeRouting && mode != config.CodebuddyVisionModePreprocess {
		return codebuddyVisionPassThrough
	}
	if !hasImage {
		return codebuddyVisionPassThrough
	}
	// The vision engine model itself must never be re-routed (avoids recursion).
	if strings.EqualFold(strings.TrimSpace(currentModel), strings.TrimSpace(visionModel)) {
		return codebuddyVisionPassThrough
	}
	// Models that natively accept images are left to the backend.
	if currentSupportsImages {
		return codebuddyVisionPassThrough
	}
	if mode == config.CodebuddyVisionModePreprocess {
		return codebuddyVisionPreprocess
	}
	return codebuddyVisionRoute
}

// applyCodebuddyVisionProxy is the single entry point that both Execute and
// ExecuteStream call after image normalization. It returns the (possibly
// rewritten) request body and reports whether the request was rewritten.
//
// Routing mode swaps the model and returns immediately. Preprocess mode performs
// an extra upstream call to the vision model to describe the images, then swaps
// the image parts for the returned description; on failure it degrades to
// omitting the images rather than failing the whole request.
func (e *CodebuddyExecutor) applyCodebuddyVisionProxy(ctx context.Context, auth *cliproxyauth.Auth, body []byte, baseModel string) ([]byte, bool) {
	visionCfg := e.cfg.CodebuddyVision
	mode := visionCfg.NormalizedVisionMode()
	if mode == config.CodebuddyVisionModeOff {
		return body, false
	}
	if !codebuddyChatHasImageInput(body) {
		return body, false
	}

	visionModel := visionCfg.VisionModel()
	currentModel := strings.TrimSpace(gjson.GetBytes(body, "model").String())
	if currentModel == "" {
		currentModel = strings.TrimSpace(baseModel)
	}

	action := codebuddyVisionPlan(mode, visionModel, currentModel, true, registry.CodebuddyModelSupportsImages(currentModel))
	switch action {
	case codebuddyVisionRoute:
		log.Infof("codebuddy vision proxy: routing %s -> %s", currentModel, visionModel)
		return rewriteCodebuddyModel(body, visionModel), true

	case codebuddyVisionPreprocess:
		description, err := e.describeImagesWithVisionModel(ctx, auth, body, visionModel, visionCfg.PreprocessPrompt)
		if err != nil {
			log.Warnf("codebuddy vision proxy: preprocess failed for %s (vision=%s): %v; omitting images", currentModel, visionModel, err)
			return replaceCodebuddyImagesWithText(body, codebuddyOmittedImageText), true
		}
		log.Infof("codebuddy vision proxy: preprocessed images for %s via %s (%d chars)", currentModel, visionModel, len(description))
		return replaceCodebuddyImagesWithText(body, description), true

	default:
		return body, false
	}
}

// describeImagesWithVisionModel sends the request body (with its images) to the
// vision model and returns a text description of the images, for preprocess mode.
func (e *CodebuddyExecutor) describeImagesWithVisionModel(ctx context.Context, auth *cliproxyauth.Auth, body []byte, visionModel, prompt string) (string, error) {
	if prompt == "" {
		prompt = defaultCodebuddyVisionPrompt
	}

	descBody := rewriteCodebuddyModel(body, visionModel)
	descBody, err := prependCodebuddySystemMessage(descBody, prompt)
	if err != nil {
		return "", err
	}
	descBody, err = sjson.SetBytes(descBody, "stream", true)
	if err != nil {
		return "", err
	}
	descBody, err = sjson.SetBytes(descBody, "stream_options.include_usage", true)
	if err != nil {
		return "", err
	}

	creds := codebuddy.CredsFromAuth(auth)
	url := creds.ResolveBaseURL() + codebuddy.ChatPath
	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(descBody))
	if err != nil {
		return "", err
	}
	applyCodebuddyHeaders(httpReq, creds)
	httpReq.Header.Set("Accept", "text/event-stream")

	httpClient := helps.NewProxyAwareHTTPClient(ctx, e.cfg, auth, 0)
	httpResp, err := httpClient.Do(httpReq)
	if err != nil {
		return "", err
	}
	defer func() { _ = httpResp.Body.Close() }()
	if httpResp.StatusCode < 200 || httpResp.StatusCode >= 300 {
		b, _ := io.ReadAll(httpResp.Body)
		return "", statusErr{code: httpResp.StatusCode, msg: string(b)}
	}

	lines := make([][]byte, 0, 64)
	scanner := bufio.NewScanner(httpResp.Body)
	scanner.Buffer(nil, 52_428_800)
	for scanner.Scan() {
		lines = append(lines, bytes.Clone(scanner.Bytes()))
	}
	if errScan := scanner.Err(); errScan != nil {
		return "", errScan
	}

	aggregated := collectChatCompletion(lines)
	content := strings.TrimSpace(gjson.GetBytes(aggregated, "choices.0.message.content").String())
	if content == "" {
		return "", fmt.Errorf("vision model returned empty description")
	}
	return content, nil
}

// prependCodebuddySystemMessage inserts a system message at the front of the
// request's messages array.
func prependCodebuddySystemMessage(body []byte, prompt string) ([]byte, error) {
	systemJSON, err := json.Marshal(map[string]any{"role": "system", "content": prompt})
	if err != nil {
		return nil, err
	}

	messages := gjson.GetBytes(body, "messages")
	if !messages.IsArray() {
		return sjson.SetRawBytes(body, "messages", append([]byte("["), append(systemJSON, ']')...))
	}

	var sb strings.Builder
	sb.WriteByte('[')
	sb.Write(systemJSON)
	for _, msg := range messages.Array() {
		sb.WriteByte(',')
		sb.WriteString(msg.Raw)
	}
	sb.WriteByte(']')
	return sjson.SetRawBytes(body, "messages", []byte(sb.String()))
}

package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"mime"
	"mime/multipart"
	"net/http"
	"net/textproto"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/google/uuid"
	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/proxyutil"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

type imageAPIUpstreamSpec struct {
	BaseURL  string   `json:"baseUrl"`
	APIKey   string   `json:"apiKey"`
	Model    string   `json:"model,omitempty"`
	ModelIDs []string `json:"modelIds"`
	ProxyURL string   `json:"proxyUrl,omitempty"`
}

// Images API calls wait for actual generation, but are bounded and cancelled
// with the downstream request. Never retry a submitted generation automatically.
var imageAPIRequestTimeout = 5 * time.Minute

const maxImageAPIResponseBytes int64 = 96 << 20

func imageAPIHasModel(upstream *imageAPIUpstreamSpec, model string) bool {
	return selectedImageAPIModel(upstream, model) != ""
}

// Families have an exact boundary: 2.5-flare belongs to 2.5, while 2.50
// does not. Suffix models are selected explicitly rather than inferred from
// catalog order, which can change whenever the provider refreshes its models.
func imageAPIModelFamily(model string) string {
	model = strings.TrimSpace(model)
	for _, family := range []string{defaultImagesToolModel, legacyImagesToolModel} {
		if model == family || (strings.HasPrefix(model, family+"-") && strings.TrimSpace(strings.TrimPrefix(model, family+"-")) != "") {
			return family
		}
	}
	return ""
}

func isOAuthImageModel(model string) bool {
	return model == defaultImagesToolModel || model == legacyImagesToolModel
}

func selectedImageAPIModel(upstream *imageAPIUpstreamSpec, configuredModel string) string {
	if upstream == nil || strings.TrimSpace(upstream.BaseURL) == "" || strings.TrimSpace(upstream.APIKey) == "" {
		return ""
	}
	configuredModel = strings.TrimSpace(configuredModel)
	if configuredModel == "" {
		return ""
	}
	model := strings.TrimSpace(upstream.Model)
	if model == "" {
		// An omitted model means the exact configured ID, never the first catalog entry.
		model = configuredModel
	}
	if model != configuredModel && !(isOAuthImageModel(configuredModel) && imageAPIModelFamily(model) == configuredModel) {
		// Only old bare GPT configurations may retain an explicitly selected suffix.
		// Complete/custom IDs always require exact agreement with the saved source.
		return ""
	}
	for _, supported := range upstream.ModelIDs {
		if strings.TrimSpace(supported) == model {
			return model
		}
	}
	return ""
}

func (s *relayServer) imageAccountAuth(account *accountSpec) (*coreauth.Auth, bool) {
	if account != nil && s.authManager != nil {
		for _, auth := range s.authManager.List() {
			if candidate := accountForAuthInManifest(s.manifest, auth); candidate != nil && candidate.ID == account.ID {
				return auth, true
			}
		}
	}
	if account == nil {
		return nil, false
	}
	return &coreauth.Auth{ID: account.ID, Provider: "codex", Attributes: map[string]string{"account_id": account.ID}}, false
}

func (s *relayServer) imageAccountEligible(account *accountSpec, model string) bool {
	if account == nil {
		return false
	}
	configuredModel := model
	if upstream := account.ImageGenerationUpstream; upstream != nil {
		model = selectedImageAPIModel(upstream, model)
		if model == "" {
			return false
		}
	}
	if account.ImageGenerationUpstream == nil && !isOAuthImageModel(model) {
		return false
	}
	now := time.Now()
	if account.ImageGenerationUpstream == nil && !imageGenerationAllowedForAccount(account) {
		return false
	}
	if strings.EqualFold(account.ImageGenerationPolicy, "disabled") || accountQuotaExhausted(s.manifest, account, now) || quotaReserveBlockReason(account, now) != "" {
		return false
	}
	auth, _ := s.imageAccountAuth(account)
	if !authAvailable(auth, model, now) || authModelExcluded(s.manifest, auth, model) ||
		(model != configuredModel && authModelExcluded(s.manifest, auth, configuredModel)) {
		return false
	}
	if upstream := account.ImageGenerationUpstream; upstream != nil {
		key := providerGatewayBackoffKey{auth.ID, strings.TrimRight(upstream.BaseURL, "/"), model}
		if s.providerBackoff.remaining(key, now) > 0 {
			return false
		}
	}
	return true
}

// Keep API accounts out of the OAuth executor, even when they lack the requested
// image model. Image selection is independent of the conversation's main model.
func (s *relayServer) prepareImageSource(c *gin.Context, model string) (*accountSpec, func(), error) {
	spec, _ := c.Request.Context().Value(clientAPIKeyContextKey).(*apiKeySpec)
	ids := imageGenerationAccountIDsForSpec(spec)
	if s.manifest == nil || len(ids) == 0 {
		if !isOAuthImageModel(model) {
			return nil, func() {}, relayStatusError{status: http.StatusNotFound, message: "no selected image API account supports model " + model}
		}
		return nil, func() {}, nil
	}
	var oauthIDs []string
	var candidates []*accountSpec
	for _, id := range ids {
		account := s.manifest.accountByID[id]
		if !s.imageAccountEligible(account, model) {
			continue
		}
		if !strings.EqualFold(account.AuthKind, "api_key") && account.ImageGenerationUpstream == nil {
			oauthIDs = append(oauthIDs, id)
			candidates = append(candidates, account)
		} else if imageAPIHasModel(account.ImageGenerationUpstream, model) && !strings.EqualFold(account.ImageGenerationPolicy, "disabled") {
			candidates = append(candidates, account)
		}
	}
	if len(candidates) == 0 {
		return nil, func() {}, relayStatusError{status: http.StatusNotFound, message: "no selected image account supports model " + model}
	}
	// Prefer the least busy source; retain configured order when equally available.
	selected := candidates[0]
	if s.policy != nil && s.policy.tracker != nil {
		load := func(a *accountSpec) (int, bool) {
			if a == nil {
				return 0, false
			}
			auth, _ := s.imageAccountAuth(a)
			id := auth.ID
			images := s.policy.tracker.imageInFlightCount(id)
			imageFull := s.manifest.MaxConcurrentImageRequests > 0 && images >= s.manifest.MaxConcurrentImageRequests
			accountFull := s.manifest.MaxAccountConcurrency > 0 && s.policy.tracker.accountInFlightCount(id) >= s.manifest.MaxAccountConcurrency
			return images, imageFull || accountFull
		}
		for _, a := range candidates[1:] {
			candidateLoad, candidateFull := load(a)
			selectedLoad, selectedFull := load(selected)
			if (!candidateFull && selectedFull) || (candidateFull == selectedFull && candidateLoad < selectedLoad) {
				selected = a
			}
		}
	}
	scoped := *spec
	scoped.ProviderGateway = nil
	scoped.ModelRouting = nil
	if selected != nil && selected.ImageGenerationUpstream != nil {
		scoped.ImageGenerationAccountIDs = []string{selected.ID}
	} else {
		scoped.ImageGenerationAccountIDs = oauthIDs
	}
	original := c.Request
	ctx := context.WithValue(original.Context(), clientAPIKeyContextKey, &scoped)
	ctx = context.WithValue(ctx, requestKindContextKey, "image_generation")
	c.Request = original.WithContext(ctx)
	return selected, func() { c.Request = original }, nil
}

// A tool declaration alone is not a generation request: normal conversation
// often advertises image_generation among many optional tools.
func forcedImageTool(body []byte) (map[string]any, bool) {
	var payload map[string]any
	if json.Unmarshal(body, &payload) != nil {
		return nil, false
	}
	choice, _ := payload["tool_choice"].(map[string]any)
	if stringField(choice, "type") != "image_generation" {
		return nil, false
	}
	tools, _ := payload["tools"].([]any)
	for _, item := range tools {
		if tool, ok := item.(map[string]any); ok && stringField(tool, "type") == "image_generation" {
			return tool, true
		}
	}
	return nil, false
}

func (s *relayServer) handleImageAPIResponses(c *gin.Context, spec *apiKeySpec, body []byte, sourceFormat sdktranslator.Format, fixedAlt string) bool {
	if s.manifest == nil || fixedAlt != "" || !sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAIResponse) {
		return false
	}
	tool, forced := forcedImageTool(body)
	if !forced {
		return false
	}
	hasAPISource := !isOAuthImageModel(configuredImagesToolModel(s.manifest))
	for _, id := range imageGenerationAccountIDsForSpec(spec) {
		if account := s.manifest.accountByID[id]; account == nil ||
			account.ImageGenerationUpstream != nil || strings.EqualFold(account.AuthKind, "api_key") {
			hasAPISource = true
			break
		}
	}
	if !hasAPISource {
		return false
	} // Preserve the original OAuth-only Responses flow.
	// The profile's image model determines account eligibility and overrides the
	// legacy image model that some Codex versions put in their tool declarations.
	tool["model"] = configuredImagesToolModel(s.manifest)
	var payload map[string]any
	_ = json.Unmarshal(body, &payload)
	tools, _ := payload["tools"].([]any)
	for i, item := range tools {
		if obj, ok := item.(map[string]any); ok && stringField(obj, "type") == "image_generation" {
			tools[i] = tool
		}
	}
	body, _ = json.Marshal(payload)
	selected, restore, err := s.prepareImageSource(c, stringField(tool, "model"))
	defer restore()
	if err != nil {
		s.writeExecutorError(c, err)
		return true
	}
	if selected != nil && selected.ImageGenerationUpstream != nil {
		s.handleImageAPIUpstream(c, selected, body, nil)
		return true
	}
	mainModel := configuredImagesMainModel(s.manifest)
	body = rewriteProviderGatewayBodyModel(body, mainModel)
	if requestBodyStream(body) {
		s.handleStream(c, body, mainModel, sourceFormat, "", []string{"codex"})
	} else {
		s.handleNonStream(c, body, mainModel, sourceFormat, "", []string{"codex"})
	}
	return true
}

func imageAPIInput(payload map[string]any) (string, []string) {
	if text, ok := payload["input"].(string); ok {
		return strings.TrimSpace(text), nil
	}
	// Use the latest user message, not prior prompts or generated images replayed
	// in the conversation history. Developer instructions are not image prompts.
	items, _ := payload["input"].([]any)
	for i := len(items) - 1; i >= 0; i-- {
		item, ok := items[i].(map[string]any)
		if !ok || stringField(item, "role") != "user" {
			continue
		}
		if text, ok := item["content"].(string); ok {
			return strings.TrimSpace(text), nil
		}
		var texts, images []string
		content, _ := item["content"].([]any)
		for _, entry := range content {
			obj, ok := entry.(map[string]any)
			if !ok {
				continue
			}
			switch stringField(obj, "type") {
			case "input_text", "text":
				texts = append(texts, stringField(obj, "text"))
			case "input_image":
				images = append(images, stringField(obj, "image_url"))
			}
		}
		return strings.TrimSpace(strings.Join(texts, "\n")), images
	}
	return "", nil
}

func imageAPIClient(upstream *imageAPIUpstreamSpec) (*http.Client, func(), error) {
	client := &http.Client{CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	close := func() {}
	if strings.TrimSpace(upstream.ProxyURL) != "" {
		transport, _, err := proxyutil.BuildHTTPTransport(upstream.ProxyURL)
		if err != nil {
			return nil, close, err
		}
		if transport == nil {
			return nil, close, fmt.Errorf("image API proxy transport unavailable")
		}
		client.Transport = transport
		close = transport.CloseIdleConnections
	}
	return client, close, nil
}

func imageAPIUpload(ctx context.Context, client *http.Client, image string) ([]byte, string, error) {
	if strings.HasPrefix(image, "data:") {
		comma := strings.IndexByte(image, ',')
		if comma < 0 || !strings.HasSuffix(image[:comma], ";base64") {
			return nil, "", fmt.Errorf("image must be a base64 data URL")
		}
		if int64(len(image[comma+1:])) > (maxImageUploadBytes*4/3)+4 {
			return nil, "", fmt.Errorf("image exceeds upload limit")
		}
		data, err := base64.StdEncoding.DecodeString(image[comma+1:])
		if err != nil {
			return nil, "", err
		}
		contentType, err := validateImageAPIBytes(data)
		return data, contentType, err
	}
	if !strings.HasPrefix(image, "https://") && !strings.HasPrefix(image, "http://") {
		return nil, "", fmt.Errorf("image must be a data URL or HTTP URL")
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, image, nil)
	if err != nil {
		return nil, "", err
	}
	// Reference image downloads never carry the upstream API credential.
	resp, err := client.Do(req)
	if err != nil {
		return nil, "", err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, "", fmt.Errorf("reference image download returned HTTP %d", resp.StatusCode)
	}
	data, err := io.ReadAll(io.LimitReader(resp.Body, maxImageUploadBytes+1))
	if int64(len(data)) > maxImageUploadBytes {
		return nil, "", fmt.Errorf("image exceeds upload limit")
	}
	if err != nil {
		return nil, "", err
	}
	contentType, err := validateImageAPIBytes(data)
	return data, contentType, err
}

func imageAPIRequestBody(ctx context.Context, client *http.Client, body []byte) ([]byte, string, string, error) {
	var payload map[string]any
	if err := json.Unmarshal(body, &payload); err != nil {
		return nil, "", "", err
	}
	tool, ok := forcedImageTool(body)
	if !ok {
		return nil, "", "", fmt.Errorf("image_generation tool is required")
	}
	prompt, images := imageAPIInput(payload)
	if prompt == "" {
		return nil, "", "", fmt.Errorf("image prompt is required")
	}
	fields := map[string]any{"model": stringField(tool, "model"), "prompt": prompt, "n": 1, "response_format": "b64_json"}
	for _, name := range []string{"size", "quality", "background", "output_format", "output_compression", "moderation", "input_fidelity"} {
		if value, exists := tool[name]; exists {
			fields[name] = value
		}
	}
	action := stringField(tool, "action")
	if action != "edit" && len(images) == 0 {
		data, err := json.Marshal(fields)
		return data, "application/json", imagesGenerationsPath, err
	}
	if len(images) == 0 {
		return nil, "", "", fmt.Errorf("image edit requires an input image")
	}
	var buf bytes.Buffer
	writer := multipart.NewWriter(&buf)
	for name, value := range fields {
		if err := writer.WriteField(name, fmt.Sprint(value)); err != nil {
			return nil, "", "", err
		}
	}
	add := func(name, image string) error {
		data, contentType, err := imageAPIUpload(ctx, client, image)
		if err != nil {
			return err
		}
		if len(data) == 0 {
			return fmt.Errorf("input image is empty")
		}
		if !strings.HasPrefix(contentType, "image/") {
			contentType = http.DetectContentType(data)
		}
		extension := ".png"
		if exts, _ := mime.ExtensionsByType(contentType); len(exts) > 0 {
			extension = exts[0]
		}
		header := make(textproto.MIMEHeader)
		header.Set("Content-Disposition", fmt.Sprintf(`form-data; name=%q; filename=%q`, name, "image"+extension))
		header.Set("Content-Type", contentType)
		part, err := writer.CreatePart(header)
		if err != nil {
			return err
		}
		_, err = part.Write(data)
		return err
	}
	for _, image := range images {
		if err := add("image[]", image); err != nil {
			return nil, "", "", err
		}
	}
	if mask, ok := tool["input_image_mask"].(map[string]any); ok {
		if value := stringField(mask, "image_url"); value != "" {
			if err := add("mask", value); err != nil {
				return nil, "", "", err
			}
		}
	}
	if err := writer.Close(); err != nil {
		return nil, "", "", err
	}
	return buf.Bytes(), writer.FormDataContentType(), imagesEditsPath, nil
}

func (s *relayServer) handleImageAPIUpstream(c *gin.Context, account *accountSpec, body []byte, direct *imageRelayRequest) {
	upstream := account.ImageGenerationUpstream
	configuredModel := configuredImagesToolModel(s.manifest)
	model := selectedImageAPIModel(upstream, configuredModel)
	if model == "" {
		writeAPIError(c, http.StatusNotFound, "selected image API model is not available", "model_not_available")
		return
	}
	var payload map[string]any
	if err := json.Unmarshal(body, &payload); err != nil {
		writeAPIError(c, http.StatusBadRequest, err.Error(), "invalid_request")
		return
	}
	tools, _ := payload["tools"].([]any)
	for _, entry := range tools {
		if tool, ok := entry.(map[string]any); ok && stringField(tool, "type") == "image_generation" {
			tool["model"] = model
		}
	}
	body, _ = json.Marshal(payload)
	ctx, cancel := context.WithTimeout(relayContext(c), imageAPIRequestTimeout)
	defer cancel()
	client, closeClient, err := imageAPIClient(upstream)
	if err != nil {
		s.writeExecutorError(c, err)
		return
	}
	defer closeClient()
	downloadClient, err := imageDownloadClient(upstream, s.imageDownloads)
	if err != nil {
		s.writeExecutorError(c, err)
		return
	}
	raw, contentType, path, err := imageAPIRequestBody(ctx, downloadClient, body)
	if err != nil {
		if ctx.Err() != nil {
			s.writeImageAPIExecutorError(c, ctx, err)
			return
		}
		writeAPIError(c, http.StatusBadRequest, err.Error(), "invalid_request")
		return
	}
	endpoint, err := providerGatewayURL(upstream.BaseURL, path)
	if err != nil {
		s.writeExecutorError(c, err)
		return
	}
	auth, registered := s.imageAccountAuth(account)
	key := providerGatewayBackoffKey{auth.ID, strings.TrimRight(upstream.BaseURL, "/"), model}
	release, admitted := s.admitProviderGateway(c, key)
	if !admitted {
		return
	}
	defer release()
	requestID := internallogging.GetRequestID(c.Request.Context())
	if s.policy != nil && s.policy.tracker != nil {
		tracker := s.policy.tracker
		if !tracker.tryReserveImageJob(requestID, auth.ID, s.manifest.MaxConcurrentImageRequests) {
			writeAPIError(c, http.StatusTooManyRequests, "image account concurrency limit exceeded", "image_concurrency_exceeded")
			return
		}
		defer tracker.releaseImageJobs(requestID)
		tracker.recordSelectedAccount(requestID, account, auth.ID)
	}
	status := http.StatusBadGateway
	succeeded := false
	defer func() {
		if registered && ctx.Err() == nil {
			result := coreauth.Result{AuthID: auth.ID, CredentialVersion: auth.CredentialVersion, RegistrationEpoch: auth.RegistrationEpoch, Provider: auth.Provider, Model: model, Success: succeeded}
			if !succeeded {
				result.Error = &coreauth.Error{Code: "image_api_error", Message: "image API request failed", HTTPStatus: status}
			}
			s.authManager.MarkResult(ctx, result)
		}
	}()
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, bytes.NewReader(raw))
	if err != nil {
		s.writeExecutorError(c, err)
		return
	}
	req.Header.Set("Authorization", "Bearer "+upstream.APIKey)
	req.Header.Set("Content-Type", contentType)
	req.Header.Set("Accept", "application/json")
	observeDirectGatewayRequest(ctx, account.ID, model, req.Header, raw)
	resp, err := client.Do(req)
	if err != nil {
		s.writeImageAPIExecutorError(c, ctx, err)
		return
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		status = resp.StatusCode
	}
	observeDirectGatewayResponse(ctx, resp)
	s.providerBackoff.observe(key, resp.StatusCode, resp.Header.Get("Retry-After"), time.Now())
	output, err := io.ReadAll(io.LimitReader(resp.Body, maxImageAPIResponseBytes+1))
	if err != nil {
		s.writeImageAPIExecutorError(c, ctx, err)
		return
	}
	if int64(len(output)) > maxImageAPIResponseBytes {
		writeAPIError(c, http.StatusBadGateway, "image API response exceeds size limit", "bad_gateway")
		return
	}
	writeUpstreamHeaders(c.Writer.Header(), resp.Header)
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		_ = c.Error(fmt.Errorf("image API returned HTTP %d", resp.StatusCode))
		c.Data(resp.StatusCode, "application/json", output)
		return
	}
	var result map[string]any
	if err = json.Unmarshal(output, &result); err != nil {
		writeAPIError(c, http.StatusBadGateway, "image API returned invalid JSON", "bad_gateway")
		return
	}
	// Generation has already consumed tokens. Record them before any local
	// download/validation or downstream cancellation can fail delivery. Final
	// request status is applied separately by requestPolicy.finalize.
	if s.policy != nil && s.policy.tracker != nil {
		detail := helps.ParseOpenAIUsage(output)
		s.policy.tracker.record(usagePayload{Type: "usage", RequestID: requestID, Provider: "openai", Model: configuredModel, UpstreamModel: model, AccountID: account.ID, AccountEmail: account.Email, AuthID: auth.ID, Success: true, RequestKind: requestKindFromPath(path), Usage: usageDetails{InputTokens: detail.InputTokens, OutputTokens: detail.OutputTokens, CachedTokens: detail.CachedTokens, ReasoningTokens: detail.ReasoningTokens, TotalTokens: detail.TotalTokens, TokenBreakdown: detail.TokenBreakdown}})
	}
	data, _ := result["data"].([]any)
	var items []any
	for _, entry := range data {
		image, ok := entry.(map[string]any)
		if !ok {
			continue
		}
		b64 := stringField(image, "b64_json")
		if b64 == "" && stringField(image, "url") != "" {
			bytes, _, downloadErr := imageAPIUpload(ctx, downloadClient, stringField(image, "url"))
			if downloadErr != nil {
				s.writeImageAPIExecutorError(c, ctx, downloadErr)
				return
			}
			b64 = base64.StdEncoding.EncodeToString(bytes)
		}
		if b64 == "" {
			continue
		}
		if int64(len(b64)) > (maxImageUploadBytes*4/3)+4 {
			writeAPIError(c, http.StatusBadGateway, "image API image exceeds size limit", "bad_gateway")
			return
		}
		decoded, err := base64.StdEncoding.DecodeString(b64)
		if err != nil {
			writeAPIError(c, http.StatusBadGateway, "image API returned invalid base64", "bad_gateway")
			return
		}
		actualContentType, err := validateImageAPIBytes(decoded)
		if err != nil {
			writeAPIError(c, http.StatusBadGateway, err.Error(), "bad_gateway")
			return
		}
		item := map[string]any{"type": "image_generation_call", "id": "ig_" + uuid.NewString(), "status": "completed", "result": b64, "model": model}
		for _, name := range []string{"revised_prompt", "output_format", "size", "quality", "background"} {
			if value, ok := image[name]; ok {
				item[name] = value
			} else if value, ok := result[name]; ok {
				item[name] = value
			}
		}
		item["output_format"] = strings.TrimPrefix(actualContentType, "image/")
		items = append(items, item)
	}
	if len(items) == 0 {
		writeAPIError(c, http.StatusBadGateway, "image API did not return b64_json image output", "bad_gateway")
		return
	}
	usage := result["usage"]
	succeeded = true
	response := map[string]any{"id": "resp_" + uuid.NewString(), "object": "response", "created_at": time.Now().Unix(), "status": "completed", "model": requestBodyModel(body), "output": items, "tool_usage": map[string]any{"image_gen": usage}}
	if direct != nil {
		results, _, created := extractImageResults(map[string]any{"response": response})
		if direct.stream {
			setEventStreamHeaders(c.Writer.Header())
			c.Status(http.StatusOK)
			forwardImageResponseFrame(mustImageAPIEvent("response.completed", response), *direct, func(name string, event []byte) {
				_, _ = fmt.Fprintf(c.Writer, "event: %s\ndata: %s\n\n", name, event)
				c.Writer.Flush()
			}, func(err error) { writeImagesStreamError(c, c.Writer, err) })
			return
		}
		out, err := buildImagesAPIResponse(results, usage, created, direct.responseFormat)
		if err != nil {
			s.writeExecutorError(c, err)
			return
		}
		c.Data(http.StatusOK, "application/json", out)
		return
	}
	if !requestBodyStream(body) {
		c.JSON(http.StatusOK, response)
		return
	}
	setEventStreamHeaders(c.Writer.Header())
	c.Status(http.StatusOK)
	sequence := 0
	emit := func(typ string, event map[string]any) {
		event["type"] = typ
		event["sequence_number"] = sequence
		sequence++
		raw, _ := json.Marshal(event)
		_, _ = fmt.Fprintf(c.Writer, "event: %s\ndata: %s\n\n", typ, raw)
		c.Writer.Flush()
	}
	pending := map[string]any{"id": response["id"], "object": "response", "created_at": response["created_at"], "status": "in_progress", "model": response["model"], "output": []any{}}
	emit("response.created", map[string]any{"response": pending})
	emit("response.in_progress", map[string]any{"response": pending})
	for index, entry := range items {
		item := entry.(map[string]any)
		started := map[string]any{"type": "image_generation_call", "id": item["id"], "status": "in_progress"}
		emit("response.output_item.added", map[string]any{"output_index": index, "item": started})
		emit("response.image_generation_call.completed", map[string]any{"output_index": index, "item_id": item["id"]})
		emit("response.output_item.done", map[string]any{"output_index": index, "item": item})
	}
	emit("response.completed", map[string]any{"response": response})
}

func (s *relayServer) writeImageAPIExecutorError(c *gin.Context, ctx context.Context, err error) {
	if ctx.Err() == context.DeadlineExceeded {
		err = relayTimeoutError{phase: "image_api", timeout: imageAPIRequestTimeout}
	}
	s.writeExecutorError(c, err)
}

func mustImageAPIEvent(typ string, response map[string]any) []byte {
	raw, _ := json.Marshal(map[string]any{"type": typ, "response": response})
	return append(append([]byte("data: "), raw...), []byte("\n\n")...)
}

func imageRequestCount(payload map[string]any) int64 {
	count, _ := numericField(payload["n"])
	return count
}

package images

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/handlers/common"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/middleware"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/scheduler"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/gin-gonic/gin"
)

const (
	operationGenerations = "generations"
	operationEdits       = "edits"
	operationVariations  = "variations"
)

// Handler Images API 代理处理器
func Handler(
	envCfg *config.EnvConfig,
	cfgManager *config.ConfigManager,
	channelScheduler *scheduler.ChannelScheduler,
) gin.HandlerFunc {
	return gin.HandlerFunc(func(c *gin.Context) {
		middleware.ProxyAuthMiddleware(envCfg)(c)
		if c.IsAborted() {
			return
		}

		operation := extractOperation(c.Request.URL.Path)
		if operation == "" {
			imagesErrorResponse(c, http.StatusNotFound, "Images endpoint not found", "invalid_request_error", "endpoint_not_found")
			return
		}

		startTime := time.Now()
		bodyBytes, err := common.ReadRequestBody(c, envCfg.MaxRequestBodySize)
		if err != nil {
			return
		}
		c.Set("requestBodyBytes", bodyBytes)

		contentType := c.GetHeader("Content-Type")
		model, isStream, ok := parseImagesRequest(c, bodyBytes, contentType, operation)
		if !ok {
			return
		}

		userID := utils.ExtractUnifiedSessionID(c, bodyBytes)
		common.LogOriginalRequest(c, bodyBytes, envCfg, "Images")

		if channelScheduler.IsMultiChannelMode(scheduler.ChannelKindImages) {
			handleMultiChannel(c, envCfg, cfgManager, channelScheduler, bodyBytes, model, userID, startTime, operation, contentType, isStream)
		} else {
			handleSingleChannel(c, envCfg, cfgManager, channelScheduler, bodyBytes, model, startTime, operation, contentType, isStream)
		}
	})
}

func extractOperation(path string) string {
	if strings.Contains(path, "/images/generations") {
		return operationGenerations
	}
	if strings.Contains(path, "/images/edits") {
		return operationEdits
	}
	if strings.Contains(path, "/images/variations") {
		return operationVariations
	}
	return ""
}

func sanitizeDiagnosticError(err error) string {
	if err == nil {
		return ""
	}
	msg := strings.TrimSpace(err.Error())
	msg = strings.ReplaceAll(msg, "\n", " ")
	msg = strings.ReplaceAll(msg, "\r", " ")
	msg = strings.Join(strings.Fields(msg), " ")
	if len(msg) > 200 {
		msg = msg[:200]
	}
	return msg
}

func logImagesValidationFailure(c *gin.Context, operation string, contentType string, bodyBytes []byte, stage string, reason string, err error) {
	log.Printf("[Images-Validation] operation=%s method=%s path=%s content_type=%q body_bytes=%d stage=%s reason=%s error=%q",
		operation,
		c.Request.Method,
		c.Request.URL.Path,
		contentType,
		len(bodyBytes),
		stage,
		reason,
		sanitizeDiagnosticError(err),
	)
}

func logImagesMultipartFailure(c *gin.Context, operation string, contentType string, bodyBytes []byte, err error) {
	stage, reason := describeMultipartDiagnostic(err)
	log.Printf("[Images-Multipart] operation=%s method=%s path=%s content_type=%q body_bytes=%d stage=%s reason=%s multipart=true boundary_present=%t error=%q",
		operation,
		c.Request.Method,
		c.Request.URL.Path,
		contentType,
		len(bodyBytes),
		stage,
		reason,
		hasMultipartBoundary(contentType),
		sanitizeDiagnosticError(err),
	)
}

func logImagesBuildRequestFailure(operation string, baseURL string, apiKey string, model string, contentType string, bodyBytes []byte, stage string, reason string, err error) {
	log.Printf("[Images-BuildRequest] operation=%s base_url=%q key_mask=%s model=%q content_type=%q body_bytes=%d stage=%s reason=%s error=%q",
		operation,
		baseURL,
		utils.MaskAPIKey(apiKey),
		model,
		contentType,
		len(bodyBytes),
		stage,
		reason,
		sanitizeDiagnosticError(err),
	)
}

func parseImagesRequest(c *gin.Context, bodyBytes []byte, contentType string, operation string) (string, bool, bool) {
	if operation != operationGenerations {
		if isJSONContentType(contentType) {
			var reqMap map[string]interface{}
			if len(bodyBytes) > 0 {
				if err := json.Unmarshal(bodyBytes, &reqMap); err != nil {
					logImagesValidationFailure(c, operation, contentType, bodyBytes, "parse_json", "invalid_json", err)
					imagesErrorResponse(c, http.StatusBadRequest, fmt.Sprintf("Invalid request body: %v", err), "invalid_request_error", "invalid_json")
					return "", false, false
				}
			}
			model, _ := reqMap["model"].(string)
			if strings.TrimSpace(model) == "" {
				model = "gpt-image-2"
			}
			return model, isImagesStreamRequest(c, bodyBytes, contentType), true
		}
		if isMultipartContentType(contentType) {
			if err := validateMultipartBody(bodyBytes, contentType); err != nil {
				logImagesMultipartFailure(c, operation, contentType, bodyBytes, err)
				imagesErrorResponse(c, http.StatusBadRequest, fmt.Sprintf("Invalid multipart body: %v", err), "invalid_request_error", "invalid_multipart")
				return "", false, false
			}
		} else {
			logImagesValidationFailure(c, operation, contentType, bodyBytes, "content_type", "invalid_content_type", fmt.Errorf("unsupported content type for images %s", operation))
		}
		model := extractImagesModel(bodyBytes, contentType)
		if strings.TrimSpace(model) == "" {
			model = "gpt-image-2"
		}
		return model, isImagesStreamRequest(c, bodyBytes, contentType), true
	}

	var reqMap map[string]interface{}
	if len(bodyBytes) > 0 {
		if err := json.Unmarshal(bodyBytes, &reqMap); err != nil {
			logImagesValidationFailure(c, operation, contentType, bodyBytes, "parse_json", "invalid_json", err)
			imagesErrorResponse(c, http.StatusBadRequest, fmt.Sprintf("Invalid request body: %v", err), "invalid_request_error", "invalid_json")
			return "", false, false
		}
	}

	model, _ := reqMap["model"].(string)
	if model == "" {
		logImagesValidationFailure(c, operation, contentType, bodyBytes, "validate_required", "missing_model", fmt.Errorf("model is required"))
		imagesErrorResponse(c, http.StatusBadRequest, "model is required", "invalid_request_error", "missing_parameter")
		return "", false, false
	}
	prompt, _ := reqMap["prompt"].(string)
	if prompt == "" {
		logImagesValidationFailure(c, operation, contentType, bodyBytes, "validate_required", "missing_prompt", fmt.Errorf("prompt is required"))
		imagesErrorResponse(c, http.StatusBadRequest, "prompt is required", "invalid_request_error", "missing_parameter")
		return "", false, false
	}

	return model, isImagesStreamRequest(c, bodyBytes, contentType), true
}

func handleMultiChannel(
	c *gin.Context,
	envCfg *config.EnvConfig,
	cfgManager *config.ConfigManager,
	channelScheduler *scheduler.ChannelScheduler,
	bodyBytes []byte,
	model string,
	userID string,
	startTime time.Time,
	operation string,
	contentType string,
	isStream bool,
) {
	metricsManager := channelScheduler.GetImagesMetricsManager()
	common.HandleMultiChannelFailover(
		c,
		envCfg,
		channelScheduler,
		scheduler.ChannelKindImages,
		"Images",
		userID,
		model,
		func(selection *scheduler.SelectionResult) common.MultiChannelAttemptResult {
			upstream := selection.Upstream
			channelIndex := selection.ChannelIndex
			if upstream == nil {
				return common.MultiChannelAttemptResult{}
			}

			baseURLs := upstream.GetAllBaseURLs()
			sortedURLResults := channelScheduler.GetSortedURLsForChannel(scheduler.ChannelKindImages, channelIndex, baseURLs)
			handled, successKey, successBaseURLIdx, failoverErr, usage, lastErr := common.TryUpstreamWithAllKeys(
				c,
				envCfg,
				cfgManager,
				channelScheduler,
				scheduler.ChannelKindImages,
				"Images",
				metricsManager,
				upstream,
				sortedURLResults,
				bodyBytes,
				isStream,
				func(upstream *config.UpstreamConfig, failedKeys map[string]bool) (string, error) {
					return cfgManager.GetNextImagesAPIKey(upstream, failedKeys)
				},
				func(c *gin.Context, upstreamCopy *config.UpstreamConfig, apiKey string) (*http.Request, error) {
					return buildOperationRequest(c, upstreamCopy, upstreamCopy.BaseURL, apiKey, bodyBytes, model, operation, contentType)
				},
				func(apiKey string) {
					_ = cfgManager.DeprioritizeAPIKey(apiKey)
				},
				func(url string) {
					channelScheduler.MarkURLFailure(scheduler.ChannelKindImages, channelIndex, url)
				},
				func(url string) {
					channelScheduler.MarkURLSuccess(scheduler.ChannelKindImages, channelIndex, url)
				},
				func(c *gin.Context, resp *http.Response, upstreamCopy *config.UpstreamConfig, apiKey string, actualRequestBody []byte) (*types.Usage, error) {
					return handleSuccess(c, resp, envCfg, startTime, isStream)
				},
				model,
				operation,
				selection.ChannelIndex,
				channelScheduler.GetChannelLogStore(scheduler.ChannelKindImages),
			)

			return common.MultiChannelAttemptResult{
				Handled:           handled,
				Attempted:         true,
				SuccessKey:        successKey,
				SuccessBaseURLIdx: successBaseURLIdx,
				FailoverError:     failoverErr,
				Usage:             usage,
				LastError:         lastErr,
			}
		},
		nil,
		func(ctx *gin.Context, failoverErr *common.FailoverError, lastError error) {
			handleAllChannelsFailed(ctx, failoverErr, lastError)
		},
	)
}

func handleSingleChannel(
	c *gin.Context,
	envCfg *config.EnvConfig,
	cfgManager *config.ConfigManager,
	channelScheduler *scheduler.ChannelScheduler,
	bodyBytes []byte,
	model string,
	startTime time.Time,
	operation string,
	contentType string,
	isStream bool,
) {
	upstream, channelIndex, err := cfgManager.GetCurrentImagesUpstreamWithIndex()
	if err != nil {
		imagesErrorResponse(c, http.StatusServiceUnavailable, "No Images upstream configured", "service_unavailable", "service_unavailable")
		return
	}
	if len(upstream.APIKeys) == 0 {
		imagesErrorResponse(c, http.StatusServiceUnavailable, fmt.Sprintf("No API keys configured for upstream \"%s\"", upstream.Name), "service_unavailable", "service_unavailable")
		return
	}

	metricsManager := channelScheduler.GetImagesMetricsManager()
	baseURLs := upstream.GetAllBaseURLs()
	urlResults := common.BuildDefaultURLResults(baseURLs)
	handled, _, _, lastFailoverError, _, lastError := common.TryUpstreamWithAllKeys(
		c,
		envCfg,
		cfgManager,
		channelScheduler,
		scheduler.ChannelKindImages,
		"Images",
		metricsManager,
		upstream,
		urlResults,
		bodyBytes,
		isStream,
		func(upstream *config.UpstreamConfig, failedKeys map[string]bool) (string, error) {
			return cfgManager.GetNextImagesAPIKey(upstream, failedKeys)
		},
		func(c *gin.Context, upstreamCopy *config.UpstreamConfig, apiKey string) (*http.Request, error) {
			return buildOperationRequest(c, upstreamCopy, upstreamCopy.BaseURL, apiKey, bodyBytes, model, operation, contentType)
		},
		func(apiKey string) {
			_ = cfgManager.DeprioritizeAPIKey(apiKey)
		},
		nil,
		nil,
		func(c *gin.Context, resp *http.Response, upstreamCopy *config.UpstreamConfig, apiKey string, actualRequestBody []byte) (*types.Usage, error) {
			return handleSuccess(c, resp, envCfg, startTime, isStream)
		},
		model,
		operation,
		channelIndex,
		channelScheduler.GetChannelLogStore(scheduler.ChannelKindImages),
	)
	if handled {
		return
	}

	log.Printf("[Images-Error] 所有 API密钥都失败了")
	handleAllKeysFailed(c, lastFailoverError, lastError)
}

func buildProviderRequest(
	c *gin.Context,
	upstream *config.UpstreamConfig,
	baseURL string,
	apiKey string,
	bodyBytes []byte,
	model string,
) (*http.Request, error) {
	return buildOperationRequest(c, upstream, baseURL, apiKey, bodyBytes, model, operationGenerations, "application/json")
}

func buildOperationRequest(
	c *gin.Context,
	upstream *config.UpstreamConfig,
	baseURL string,
	apiKey string,
	bodyBytes []byte,
	model string,
	operation string,
	contentType string,
) (*http.Request, error) {
	serviceType, err := config.NormalizeImagesServiceTypeForProxy(upstream.ServiceType)
	if err != nil {
		logImagesBuildRequestFailure(operation, baseURL, apiKey, model, contentType, bodyBytes, "normalize_service_type", "invalid_service_type", err)
		return nil, err
	}
	upstream.ServiceType = serviceType

	requestBody := bodyBytes
	requestContentType := contentType
	redirectedModel := config.RedirectModel(model, upstream)

	if isMultipartContentType(contentType) {
		originalModel, hasModelField := extractMultipartField(bodyBytes, contentType, "model")
		if redirectedModel != "" && (!hasModelField || strings.TrimSpace(originalModel) == "" || redirectedModel != originalModel) {
			requestBody, requestContentType, err = rewriteMultipartFormField(bodyBytes, contentType, "model", redirectedModel)
			if err != nil {
				stage, reason := describeMultipartDiagnostic(err)
				if stage == "" {
					stage = "rewrite_field"
				}
				if reason == "" {
					reason = "part_read_failed"
				}
				logImagesBuildRequestFailure(operation, baseURL, apiKey, redirectedModel, contentType, bodyBytes, stage, reason, err)
				return nil, err
			}
		}
	} else if operation == operationGenerations || len(bodyBytes) > 0 {
		requestBody, requestContentType, err = buildJSONRequestBody(bodyBytes, model, redirectedModel, operation)
		if err != nil {
			logImagesBuildRequestFailure(operation, baseURL, apiKey, redirectedModel, contentType, bodyBytes, "build_json", "invalid_json", err)
			return nil, err
		}
	}

	url := buildOperationURL(baseURL, operation)
	req, err := http.NewRequestWithContext(c.Request.Context(), http.MethodPost, url, bytes.NewReader(requestBody))
	if err != nil {
		logImagesBuildRequestFailure(operation, baseURL, apiKey, redirectedModel, requestContentType, requestBody, "new_request", "request_init_failed", err)
		return nil, err
	}
	if c.Request.URL != nil {
		req.URL.RawQuery = c.Request.URL.RawQuery
	}
	req.Header = prepareImagesUpstreamHeaders(c, req.URL.Host, requestContentType)
	utils.SetAuthenticationHeader(req.Header, apiKey)
	utils.ApplyCustomHeaders(req.Header, upstream.CustomHeaders)
	return req, nil
}

func buildOperationURL(baseURL string, operation string) string {
	skipVersionPrefix := strings.HasSuffix(baseURL, "#")
	baseURL = strings.TrimSuffix(baseURL, "#")
	baseURL = strings.TrimRight(baseURL, "/")
	if skipVersionPrefix {
		return fmt.Sprintf("%s/images/%s", baseURL, operation)
	}
	return fmt.Sprintf("%s/v1/images/%s", baseURL, operation)
}

func buildJSONRequestBody(bodyBytes []byte, model string, redirectedModel string, operation string) ([]byte, string, error) {
	var reqMap map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &reqMap); err != nil {
		if operation == operationGenerations {
			return nil, "", err
		}
		return bodyBytes, "application/json", nil
	}
	if reqMap == nil {
		reqMap = make(map[string]interface{})
	}
	if model != "" && redirectedModel != "" {
		reqMap["model"] = redirectedModel
	}
	requestBody, err := json.Marshal(reqMap)
	if err != nil {
		return nil, "", err
	}
	return requestBody, "application/json", nil
}

func prepareImagesUpstreamHeaders(c *gin.Context, targetHost string, contentType string) http.Header {
	headers := c.Request.Header.Clone()
	headers.Set("Host", targetHost)
	headers.Del("x-proxy-key")
	headers.Del("X-Forwarded-For")
	headers.Del("X-Forwarded-Host")
	headers.Del("X-Forwarded-Proto")
	headers.Del("X-Real-IP")
	headers.Del("Via")
	headers.Del("Forwarded")
	headers.Del("Accept-Encoding")
	if strings.TrimSpace(contentType) == "" {
		headers.Del("Content-Type")
	} else {
		headers.Set("Content-Type", contentType)
	}
	return headers
}

func handleSuccess(c *gin.Context, resp *http.Response, envCfg *config.EnvConfig, startTime time.Time, isStream bool) (*types.Usage, error) {
	defer resp.Body.Close()
	if isStream {
		return nil, passthroughStreamingResponseWithLog(c, resp, envCfg, startTime)
	}

	bodyBytes, err := io.ReadAll(resp.Body)
	if err != nil {
		imagesErrorResponse(c, http.StatusInternalServerError, "Failed to read response", "server_error", "server_error")
		return nil, err
	}
	if envCfg.EnableResponseLogs {
		responseTime := time.Since(startTime).Milliseconds()
		log.Printf("[Images-Timing] 响应完成: %dms, 状态: %d", responseTime, resp.StatusCode)
		common.LogUpstreamResponse(resp, bodyBytes, envCfg, "Images")
	}
	resp.Body = io.NopCloser(bytes.NewReader(bodyBytes))
	var respMap map[string]interface{}
	if err := common.PassthroughJSONResponse(c, resp, &respMap); err != nil {
		return nil, nil
	}
	if u, ok := respMap["usage"].(map[string]interface{}); ok {
		inputTokens, _ := u["input_tokens"].(float64)
		outputTokens, _ := u["output_tokens"].(float64)
		return &types.Usage{InputTokens: int(inputTokens), OutputTokens: int(outputTokens)}, nil
	}
	return nil, nil
}

func passthroughStreamingResponse(c *gin.Context, resp *http.Response) error {
	return passthroughStreamingResponseWithLog(c, resp, config.NewEnvConfig(), time.Now())
}

func passthroughStreamingResponseWithLog(c *gin.Context, resp *http.Response, envCfg *config.EnvConfig, startTime time.Time) error {
	if envCfg.EnableResponseLogs {
		responseTime := time.Since(startTime).Milliseconds()
		log.Printf("[Images-Stream] 流式响应开始: %dms, 状态: %d", responseTime, resp.StatusCode)
		common.LogUpstreamResponseHeaders(resp, envCfg, "Images")
	}

	utils.ForwardResponseHeaders(resp.Header, c.Writer)
	c.Status(resp.StatusCode)

	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		bodyBytes, err := io.ReadAll(resp.Body)
		if len(bodyBytes) > 0 {
			common.LogUpstreamResponseBody(bodyBytes, envCfg, "Images")
			if _, writeErr := c.Writer.Write(bodyBytes); writeErr != nil {
				err = writeErr
			}
		}
		if envCfg.EnableResponseLogs {
			responseTime := time.Since(startTime).Milliseconds()
			log.Printf("[Images-Stream] 流式响应完成: %dms", responseTime)
		}
		return err
	}

	buffer := make([]byte, 4*1024)
	logBuffer := common.NewLimitedLogBuffer(common.MaxUpstreamResponseLogBytes)
	streamLoggingEnabled := envCfg.EnableResponseLogs && envCfg.IsDevelopment()
	for {
		n, err := resp.Body.Read(buffer)
		if n > 0 {
			if streamLoggingEnabled {
				logBuffer.Write(buffer[:n])
			}
			if _, writeErr := c.Writer.Write(buffer[:n]); writeErr != nil {
				if common.IsClientDisconnectError(writeErr) || writeErr == io.ErrClosedPipe || strings.Contains(strings.ToLower(writeErr.Error()), "closed pipe") {
					return context.Canceled
				}
				return writeErr
			}
			flusher.Flush()
		}
		if err == io.EOF {
			if logBuffer.Len() > 0 {
				common.LogUpstreamResponseBody(logBuffer.Bytes(), envCfg, "Images")
			}
			if envCfg.EnableResponseLogs {
				responseTime := time.Since(startTime).Milliseconds()
				log.Printf("[Images-Stream] 流式响应完成: %dms", responseTime)
			}
			return nil
		}
		if err != nil {
			return err
		}
	}
}

func imagesErrorResponse(c *gin.Context, statusCode int, message, errorType, code string) {
	c.JSON(statusCode, gin.H{
		"error": gin.H{
			"message": message,
			"type":    errorType,
			"code":    code,
		},
	})
}

func handleAllChannelsFailed(c *gin.Context, failoverErr *common.FailoverError, lastError error) {
	if failoverErr != nil {
		c.Data(failoverErr.Status, "application/json", failoverErr.Body)
		return
	}
	errMsg := "All channels failed"
	if lastError != nil {
		errMsg = lastError.Error()
	}
	imagesErrorResponse(c, http.StatusServiceUnavailable, errMsg, "service_unavailable", "service_unavailable")
}

func handleAllKeysFailed(c *gin.Context, failoverErr *common.FailoverError, lastError error) {
	if failoverErr != nil {
		c.Data(failoverErr.Status, "application/json", failoverErr.Body)
		return
	}
	errMsg := "All API keys failed"
	if lastError != nil {
		errMsg = lastError.Error()
	}
	imagesErrorResponse(c, http.StatusServiceUnavailable, errMsg, "service_unavailable", "service_unavailable")
}

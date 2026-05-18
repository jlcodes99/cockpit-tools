package common

import (
	"github.com/gin-gonic/gin"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
)

func BuildChannelView(up config.UpstreamConfig, index int) gin.H {
	status := config.GetChannelStatus(&up)
	priority := config.GetChannelPriority(&up, index)
	return gin.H{
		"index":                         index,
		"name":                          up.Name,
		"serviceType":                   up.ServiceType,
		"baseUrl":                       up.BaseURL,
		"baseUrls":                      up.BaseURLs,
		"apiKeys":                       up.APIKeys,
		"description":                   up.Description,
		"website":                       up.Website,
		"insecureSkipVerify":            up.InsecureSkipVerify,
		"modelMapping":                  up.ModelMapping,
		"modelMappingExplicit":          up.ModelMappingExplicit,
		"reasoningMapping":              up.ReasoningMapping,
		"reasoningParamStyle":           up.ReasoningParamStyle,
		"textVerbosity":                 up.TextVerbosity,
		"fastMode":                      up.FastMode,
		"normalizeNonstandardChatRoles": up.NormalizeNonstandardChatRoles,
		"stripCodexClientTools":         up.IsCodexToolCompatEnabled(),
		"latency":                       nil,
		"status":                        status,
		"adminState":                    config.GetChannelAdminState(&up),
		"effectiveState":                config.GetChannelEffectiveState(&up),
		"runtimeState":                  config.GetChannelRuntimeState(&up),
		"priority":                      priority,
		"promotionUntil":                up.PromotionUntil,
		"lowQuality":                    up.LowQuality,
		"customHeaders":                 up.CustomHeaders,
		"proxyUrl":                      up.ProxyURL,
		"supportedModels":               up.SupportedModels,
		"routePrefix":                   up.RoutePrefix,
		"disabledApiKeys":               up.DisabledAPIKeys,
		"autoBlacklistBalance":          up.IsAutoBlacklistBalanceEnabled(),
		"normalizeMetadataUserId":       up.IsNormalizeMetadataUserIDEnabled(),
		"codexNativeToolPassthrough":    up.CodexNativeToolPassthrough,
		"codexToolCompat":               up.IsCodexToolCompatEnabled(),
		"noVision":                      up.NoVision,
		"noVisionModels":                up.NoVisionModels,
		"visionFallbackModel":           up.VisionFallbackModel,
	}
}

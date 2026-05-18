package config

import (
	"fmt"
	"log"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
)

// ============== Images 渠道方法 ==============

// normalizeImagesServiceType 规范化并校验 Images 渠道 serviceType。
func normalizeImagesServiceType(serviceType string) (string, error) {
	normalized := normalizeUpstreamServiceType(serviceType, "openai")
	if normalized != "openai" {
		return "", &ConfigError{Message: fmt.Sprintf("Images 渠道仅支持 openai serviceType，当前为 %s", normalized)}
	}
	return normalized, nil
}

// NormalizeImagesServiceTypeForProxy 供代理层复用 Images 渠道 serviceType 校验。
func NormalizeImagesServiceTypeForProxy(serviceType string) (string, error) {
	return normalizeImagesServiceType(serviceType)
}

// GetCurrentImagesUpstream 获取当前 Images 上游配置
// 优先选择第一个 active 状态的渠道，若无则回退到第一个渠道
func (cm *ConfigManager) GetCurrentImagesUpstream() (*UpstreamConfig, error) {
	cm.mu.RLock()
	defer cm.mu.RUnlock()

	if len(cm.config.ImagesUpstream) == 0 {
		return nil, fmt.Errorf("未配置任何 Images 渠道")
	}

	// 优先选择第一个 active 状态的渠道
	for i := range cm.config.ImagesUpstream {
		status := cm.config.ImagesUpstream[i].Status
		if status == "" || status == "active" {
			return &cm.config.ImagesUpstream[i], nil
		}
	}

	// 没有 active 渠道，回退到第一个渠道
	return &cm.config.ImagesUpstream[0], nil
}

// GetCurrentImagesUpstreamWithIndex 获取当前 Images 上游配置及其索引
func (cm *ConfigManager) GetCurrentImagesUpstreamWithIndex() (*UpstreamConfig, int, error) {
	cm.mu.RLock()
	defer cm.mu.RUnlock()

	if len(cm.config.ImagesUpstream) == 0 {
		return nil, 0, fmt.Errorf("未配置任何 Images 渠道")
	}

	for i := range cm.config.ImagesUpstream {
		status := cm.config.ImagesUpstream[i].Status
		if status == "" || status == "active" {
			return &cm.config.ImagesUpstream[i], i, nil
		}
	}

	return &cm.config.ImagesUpstream[0], 0, nil
}

// AddImagesUpstream 添加 Images 上游
func (cm *ConfigManager) AddImagesUpstream(upstream UpstreamConfig) error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	// 检查 Name 是否已存在
	for _, existing := range cm.config.ImagesUpstream {
		if existing.Name == upstream.Name {
			return fmt.Errorf("渠道名称 '%s' 已存在", upstream.Name)
		}
	}

	// 新建渠道默认设为 active
	if upstream.Status == "" {
		upstream.Status = "active"
	}

	upstream.ServiceType = normalizeUpstreamServiceType(upstream.ServiceType, "openai")

	serviceType, err := normalizeImagesServiceType(upstream.ServiceType)
	if err != nil {
		return err
	}
	upstream.ServiceType = serviceType

	// 去重 API Keys 和 Base URLs
	upstream.APIKeys = deduplicateStrings(upstream.APIKeys)
	upstream.BaseURL = utils.CanonicalBaseURL(upstream.BaseURL, upstream.ServiceType)
	upstream.BaseURLs = deduplicateBaseURLs(upstream.BaseURLs, upstream.ServiceType)

	cm.config.ImagesUpstream = append(cm.config.ImagesUpstream, upstream)

	if err := cm.saveConfigLocked(cm.config); err != nil {
		return err
	}

	log.Printf("[Config-Upstream] 已添加 Images 上游: %s", upstream.Name)
	return nil
}

// UpdateImagesUpstream 更新 Images 上游
// 返回值：shouldResetMetrics 表示是否需要重置渠道指标（熔断状态）
func (cm *ConfigManager) UpdateImagesUpstream(index int, updates UpstreamUpdate) (shouldResetMetrics bool, err error) {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	if index < 0 || index >= len(cm.config.ImagesUpstream) {
		return false, fmt.Errorf("无效的 Images 上游索引: %d", index)
	}

	// 保存修改前的配置快照用于变更检测
	originalConfig := cm.config.deepCopy()

	upstream := &cm.config.ImagesUpstream[index]
	serviceType, err := normalizeImagesServiceType(upstream.ServiceType)
	if err != nil {
		return false, err
	}
	upstream.ServiceType = serviceType
	if updates.ServiceType != nil {
		serviceType, err = normalizeImagesServiceType(*updates.ServiceType)
		if err != nil {
			return false, err
		}
	}

	if updates.Name != nil {
		upstream.Name = *updates.Name
	}
	if updates.BaseURL != nil {
		upstream.BaseURL = utils.CanonicalBaseURL(*updates.BaseURL, serviceType)
		if updates.BaseURLs == nil {
			upstream.BaseURLs = nil
		}
	}
	if updates.BaseURLs != nil {
		upstream.BaseURLs = deduplicateBaseURLs(updates.BaseURLs, serviceType)
	}
	if updates.ServiceType != nil {
		upstream.ServiceType = serviceType
	}
	if updates.Description != nil {
		upstream.Description = *updates.Description
	}
	if updates.Website != nil {
		upstream.Website = *updates.Website
	}
	if updates.APIKeys != nil {
		newKeys := make(map[string]bool)
		for _, key := range updates.APIKeys {
			newKeys[key] = true
		}

		for _, key := range upstream.APIKeys {
			if !newKeys[key] {
				alreadyInHistory := false
				for _, hk := range upstream.HistoricalAPIKeys {
					if hk == key {
						alreadyInHistory = true
						break
					}
				}
				if !alreadyInHistory {
					upstream.HistoricalAPIKeys = append(upstream.HistoricalAPIKeys, key)
					log.Printf("[Config-Upstream] Images 渠道 [%d] %s: Key %s 已移入历史列表", index, upstream.Name, utils.MaskAPIKey(key))
				}
			}
		}

		var newHistoricalKeys []string
		for _, hk := range upstream.HistoricalAPIKeys {
			if !newKeys[hk] {
				newHistoricalKeys = append(newHistoricalKeys, hk)
			} else {
				log.Printf("[Config-Upstream] Images 渠道 [%d] %s: Key %s 已从历史列表恢复", index, upstream.Name, utils.MaskAPIKey(hk))
			}
		}
		upstream.HistoricalAPIKeys = newHistoricalKeys

		wasSuspended := upstream.Status == "suspended"
		if applySingleKeyReplacementTransition(upstream, updates.APIKeys) {
			shouldResetMetrics = true
			if wasSuspended {
				log.Printf("[Config-Upstream] Images 渠道 [%d] %s 已从暂停状态自动激活（单 key 更换）", index, upstream.Name)
			}
		}
		upstream.APIKeys = deduplicateStrings(updates.APIKeys)
	}
	if updates.ModelMapping != nil {
		upstream.ModelMapping = updates.ModelMapping
	}
	if updates.ModelMappingExplicit != nil {
		upstream.ModelMappingExplicit = updates.ModelMappingExplicit
	}
	if updates.ReasoningMapping != nil {
		upstream.ReasoningMapping = updates.ReasoningMapping
	}
	if updates.ReasoningParamStyle != nil {
		upstream.ReasoningParamStyle = *updates.ReasoningParamStyle
	}
	if updates.TextVerbosity != nil {
		upstream.TextVerbosity = *updates.TextVerbosity
	}
	if updates.FastMode != nil {
		upstream.FastMode = *updates.FastMode
	}
	if updates.NormalizeNonstandardChatRoles != nil {
		upstream.NormalizeNonstandardChatRoles = *updates.NormalizeNonstandardChatRoles
	}
	if updates.InsecureSkipVerify != nil {
		upstream.InsecureSkipVerify = *updates.InsecureSkipVerify
	}
	if updates.Priority != nil {
		upstream.Priority = *updates.Priority
	}
	if updates.Status != nil {
		upstream.Status = *updates.Status
	}
	if updates.PromotionUntil != nil {
		upstream.PromotionUntil = updates.PromotionUntil
	}
	if updates.LowQuality != nil {
		upstream.LowQuality = *updates.LowQuality
	}
	if updates.AutoBlacklistBalance != nil {
		v := *updates.AutoBlacklistBalance
		upstream.AutoBlacklistBalance = &v
	}
	if updates.NormalizeMetadataUserID != nil {
		v := *updates.NormalizeMetadataUserID
		upstream.NormalizeMetadataUserID = &v
	}
	if updates.CodexNativeToolPassthrough != nil {
		upstream.CodexNativeToolPassthrough = *updates.CodexNativeToolPassthrough
	}
	if updates.CodexToolCompat != nil {
		v := *updates.CodexToolCompat
		upstream.CodexToolCompat = &v
	}
	if updates.CustomHeaders != nil {
		upstream.CustomHeaders = updates.CustomHeaders
	}
	if updates.ProxyURL != nil {
		upstream.ProxyURL = *updates.ProxyURL
	}
	if updates.SupportedModels != nil {
		upstream.SupportedModels = updates.SupportedModels
	}
	if updates.RoutePrefix != nil {
		upstream.RoutePrefix = *updates.RoutePrefix
	}
	if updates.NoVision != nil {
		upstream.NoVision = *updates.NoVision
	}
	if updates.NoVisionModels != nil {
		upstream.NoVisionModels = updates.NoVisionModels
	}
	if updates.VisionFallbackModel != nil {
		upstream.VisionFallbackModel = *updates.VisionFallbackModel
	}

	// 检测配置是否真的发生了变化
	if !cm.hasConfigChanged(originalConfig, cm.config) {
		log.Printf("[Config-Upstream] Images 渠道 [%d] %s 配置未发生实质性变化，跳过保存", index, upstream.Name)
		return shouldResetMetrics, nil
	}

	if err := cm.saveConfigLocked(cm.config); err != nil {
		return false, err
	}

	log.Printf("[Config-Upstream] 已更新 Images 上游: [%d] %s", index, cm.config.ImagesUpstream[index].Name)
	return shouldResetMetrics, nil
}

// RemoveImagesUpstream 删除 Images 上游
func (cm *ConfigManager) RemoveImagesUpstream(index int) (*UpstreamConfig, error) {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	if index < 0 || index >= len(cm.config.ImagesUpstream) {
		return nil, fmt.Errorf("无效的 Images 上游索引: %d", index)
	}

	removed := cm.config.ImagesUpstream[index]
	cm.config.ImagesUpstream = append(cm.config.ImagesUpstream[:index], cm.config.ImagesUpstream[index+1:]...)

	cm.clearFailedKeysForUpstream(&removed, "Images")

	if err := cm.saveConfigLocked(cm.config); err != nil {
		return nil, err
	}

	log.Printf("[Config-Upstream] 已删除 Images 上游: %s", removed.Name)
	return &removed, nil
}

// AddImagesAPIKey 添加 Images 上游的 API 密钥
func (cm *ConfigManager) AddImagesAPIKey(index int, apiKey string) error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	if index < 0 || index >= len(cm.config.ImagesUpstream) {
		return fmt.Errorf("无效的上游索引: %d", index)
	}

	for _, key := range cm.config.ImagesUpstream[index].APIKeys {
		if key == apiKey {
			return fmt.Errorf("API密钥已存在")
		}
	}

	cm.config.ImagesUpstream[index].APIKeys = append(cm.config.ImagesUpstream[index].APIKeys, apiKey)

	var newDisabledKeys []DisabledKeyInfo
	for _, dk := range cm.config.ImagesUpstream[index].DisabledAPIKeys {
		if dk.Key != apiKey {
			newDisabledKeys = append(newDisabledKeys, dk)
		}
	}
	cm.config.ImagesUpstream[index].DisabledAPIKeys = newDisabledKeys

	var newHistoricalKeys []string
	for _, hk := range cm.config.ImagesUpstream[index].HistoricalAPIKeys {
		if hk != apiKey {
			newHistoricalKeys = append(newHistoricalKeys, hk)
		} else {
			log.Printf("[Images-Key] 上游 [%d] %s: Key %s 已从历史列表恢复", index, cm.config.ImagesUpstream[index].Name, utils.MaskAPIKey(hk))
		}
	}
	cm.config.ImagesUpstream[index].HistoricalAPIKeys = newHistoricalKeys

	if err := cm.saveConfigLocked(cm.config); err != nil {
		return err
	}

	log.Printf("[Images-Key] 已添加API密钥到 Images 上游 [%d] %s", index, cm.config.ImagesUpstream[index].Name)
	return nil
}

// RemoveImagesAPIKey 删除 Images 上游的 API 密钥
func (cm *ConfigManager) RemoveImagesAPIKey(index int, apiKey string) error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	if index < 0 || index >= len(cm.config.ImagesUpstream) {
		return fmt.Errorf("无效的上游索引: %d", index)
	}

	keys := cm.config.ImagesUpstream[index].APIKeys
	found := false
	for i, key := range keys {
		if key == apiKey {
			cm.config.ImagesUpstream[index].APIKeys = append(keys[:i], keys[i+1:]...)
			found = true
			break
		}
	}

	if !found {
		return fmt.Errorf("API密钥不存在")
	}

	alreadyInHistory := false
	for _, hk := range cm.config.ImagesUpstream[index].HistoricalAPIKeys {
		if hk == apiKey {
			alreadyInHistory = true
			break
		}
	}
	if !alreadyInHistory {
		cm.config.ImagesUpstream[index].HistoricalAPIKeys = append(cm.config.ImagesUpstream[index].HistoricalAPIKeys, apiKey)
		log.Printf("[Images-Key] 上游 [%d] %s: Key %s 已移入历史列表", index, cm.config.ImagesUpstream[index].Name, utils.MaskAPIKey(apiKey))
	}

	if err := cm.saveConfigLocked(cm.config); err != nil {
		return err
	}

	log.Printf("[Images-Key] 已从 Images 上游 [%d] %s 删除API密钥", index, cm.config.ImagesUpstream[index].Name)
	return nil
}

// GetNextImagesAPIKey 获取下一个 Images API 密钥（纯 failover 模式）
func (cm *ConfigManager) GetNextImagesAPIKey(upstream *UpstreamConfig, failedKeys map[string]bool) (string, error) {
	return cm.GetNextAPIKey(upstream, failedKeys, "Images")
}

// MoveImagesAPIKeyToTop 将指定 Images 渠道的 API 密钥移到最前面
func (cm *ConfigManager) MoveImagesAPIKeyToTop(upstreamIndex int, apiKey string) error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	if upstreamIndex < 0 || upstreamIndex >= len(cm.config.ImagesUpstream) {
		return fmt.Errorf("无效的上游索引: %d", upstreamIndex)
	}

	upstream := &cm.config.ImagesUpstream[upstreamIndex]
	index := -1
	for i, key := range upstream.APIKeys {
		if key == apiKey {
			index = i
			break
		}
	}

	if index <= 0 {
		return nil
	}

	upstream.APIKeys = append([]string{apiKey}, append(upstream.APIKeys[:index], upstream.APIKeys[index+1:]...)...)
	return cm.saveConfigLocked(cm.config)
}

// MoveImagesAPIKeyToBottom 将指定 Images 渠道的 API 密钥移到最后面
func (cm *ConfigManager) MoveImagesAPIKeyToBottom(upstreamIndex int, apiKey string) error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	if upstreamIndex < 0 || upstreamIndex >= len(cm.config.ImagesUpstream) {
		return fmt.Errorf("无效的上游索引: %d", upstreamIndex)
	}

	upstream := &cm.config.ImagesUpstream[upstreamIndex]
	index := -1
	for i, key := range upstream.APIKeys {
		if key == apiKey {
			index = i
			break
		}
	}

	if index == -1 || index == len(upstream.APIKeys)-1 {
		return nil
	}

	upstream.APIKeys = append(upstream.APIKeys[:index], upstream.APIKeys[index+1:]...)
	upstream.APIKeys = append(upstream.APIKeys, apiKey)
	return cm.saveConfigLocked(cm.config)
}

// ReorderImagesUpstreams 重新排序 Images 渠道优先级
// order 是渠道索引数组，按新的优先级顺序排列（只更新传入的渠道，支持部分排序）
func (cm *ConfigManager) ReorderImagesUpstreams(order []int) error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	if len(order) == 0 {
		return fmt.Errorf("排序数组不能为空")
	}

	seen := make(map[int]bool)
	for _, idx := range order {
		if idx < 0 || idx >= len(cm.config.ImagesUpstream) {
			return fmt.Errorf("无效的渠道索引: %d", idx)
		}
		if seen[idx] {
			return fmt.Errorf("重复的渠道索引: %d", idx)
		}
		seen[idx] = true
	}

	for i, idx := range order {
		cm.config.ImagesUpstream[idx].Priority = i + 1
	}

	if err := cm.saveConfigLocked(cm.config); err != nil {
		return err
	}

	log.Printf("[Config-Reorder] 已更新 Images 渠道优先级顺序 (%d 个渠道)", len(order))
	return nil
}

// SetImagesChannelStatus 设置 Images 渠道状态
func (cm *ConfigManager) SetImagesChannelStatus(index int, status string) error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	if index < 0 || index >= len(cm.config.ImagesUpstream) {
		return fmt.Errorf("无效的上游索引: %d", index)
	}

	status = strings.ToLower(status)
	if status != "active" && status != "suspended" && status != "disabled" {
		return fmt.Errorf("无效的状态: %s (允许值: active, suspended, disabled)", status)
	}

	cm.config.ImagesUpstream[index].Status = status

	if status == "suspended" && cm.config.ImagesUpstream[index].PromotionUntil != nil {
		cm.config.ImagesUpstream[index].PromotionUntil = nil
		log.Printf("[Config-Status] 已清除 Images 渠道 [%d] %s 的促销期", index, cm.config.ImagesUpstream[index].Name)
	}

	if err := cm.saveConfigLocked(cm.config); err != nil {
		return err
	}

	log.Printf("[Config-Status] 已设置 Images 渠道 [%d] %s 状态为: %s", index, cm.config.ImagesUpstream[index].Name, status)
	return nil
}

// SetImagesChannelPromotion 设置 Images 渠道促销期
func (cm *ConfigManager) SetImagesChannelPromotion(index int, duration time.Duration) error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	if index < 0 || index >= len(cm.config.ImagesUpstream) {
		return fmt.Errorf("无效的 Images 上游索引: %d", index)
	}

	if duration <= 0 {
		cm.config.ImagesUpstream[index].PromotionUntil = nil
		log.Printf("[Config-Promotion] 已清除 Images 渠道 [%d] %s 的促销期", index, cm.config.ImagesUpstream[index].Name)
	} else {
		for i := range cm.config.ImagesUpstream {
			if i != index && cm.config.ImagesUpstream[i].PromotionUntil != nil {
				cm.config.ImagesUpstream[i].PromotionUntil = nil
			}
		}
		promotionEnd := time.Now().Add(duration)
		cm.config.ImagesUpstream[index].PromotionUntil = &promotionEnd
		log.Printf("[Config-Promotion] 已设置 Images 渠道 [%d] %s 进入促销期，截止: %s", index, cm.config.ImagesUpstream[index].Name, promotionEnd.Format(time.RFC3339))
	}

	return cm.saveConfigLocked(cm.config)
}

// GetPromotedImagesChannel 获取当前处于促销期的 Images 渠道索引
func (cm *ConfigManager) GetPromotedImagesChannel() (int, bool) {
	cm.mu.RLock()
	defer cm.mu.RUnlock()

	for i, upstream := range cm.config.ImagesUpstream {
		if IsChannelInPromotion(&upstream) && GetChannelStatus(&upstream) == "active" {
			return i, true
		}
	}
	return -1, false
}

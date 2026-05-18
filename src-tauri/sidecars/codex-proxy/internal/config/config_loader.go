package config

import (
	"bytes"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"time"

	"github.com/fsnotify/fsnotify"
)

const (
	maxBackups      = 10
	keyRecoveryTime = 5 * time.Minute
	maxFailureCount = 3
)

// NewConfigManager 创建配置管理器
func NewConfigManager(configFile string) (*ConfigManager, error) {
	cm := &ConfigManager{
		configFile:      configFile,
		failedKeysCache: make(map[string]*FailedKey),
		keyRecoveryTime: keyRecoveryTime,
		maxFailureCount: maxFailureCount,
		stopChan:        make(chan struct{}),
	}

	// 加载配置
	if err := cm.loadConfig(); err != nil {
		return nil, err
	}

	// 启动文件监听
	if err := cm.startWatcher(); err != nil {
		log.Printf("[Config-Watcher] 警告: 启动配置文件监听失败: %v", err)
	}

	// 启动定期清理
	go cm.cleanupExpiredFailures()

	return cm, nil
}

// loadConfig 加载配置
func (cm *ConfigManager) loadConfig() error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	// 如果配置文件不存在，创建默认配置
	if _, err := os.Stat(cm.configFile); os.IsNotExist(err) {
		return cm.createDefaultConfig()
	}

	// 读取配置文件
	data, err := os.ReadFile(cm.configFile)
	if err != nil {
		return err
	}

	if err := json.Unmarshal(data, &cm.config); err != nil {
		return err
	}

	// 兼容旧配置：检查 FuzzyModeEnabled 字段是否存在
	// 如果不存在，默认设为 true（新功能默认启用）
	needSaveDefaults := cm.applyConfigDefaults(data)
	if cm.applyServiceTypeDefaults() {
		needSaveDefaults = true
	}
	if cm.applyCodexToolCompatMigration(data) {
		needSaveDefaults = true
	}

	// 兼容旧格式：检测是否需要迁移
	needMigration := cm.migrateOldFormat()

	// 如果有默认值迁移或格式迁移，保存配置
	if needSaveDefaults || needMigration {
		if err := cm.saveConfigLocked(cm.config); err != nil {
			log.Printf("[Config-Migration] 警告: 保存迁移后的配置失败: %v", err)
			return err
		}
		if needMigration {
			log.Printf("[Config-Migration] 配置迁移完成")
		}
	}

	// 自检：没有配置 key 的渠道自动暂停
	if cm.validateChannelKeys() {
		if err := cm.saveConfigLocked(cm.config); err != nil {
			log.Printf("[Config-Validate] 警告: 保存自检后的配置失败: %v", err)
			return err
		}
	}

	return nil
}

// createDefaultConfig 创建默认配置
func (cm *ConfigManager) createDefaultConfig() error {
	defaultConfig := Config{
		Upstream:                 []UpstreamConfig{},
		CurrentUpstream:          0,
		ResponsesUpstream:        []UpstreamConfig{},
		CurrentResponsesUpstream: 0,
		GeminiUpstream:           []UpstreamConfig{},
		FuzzyModeEnabled:         true, // 默认启用 Fuzzy 模式
		StripBillingHeader:       true, // 默认启用移除计费头
	}

	if err := os.MkdirAll(filepath.Dir(cm.configFile), 0700); err != nil {
		return err
	}

	return cm.saveConfigLocked(defaultConfig)
}

// applyConfigDefaults 应用配置默认值
// rawJSON: 原始 JSON 数据，用于检测字段是否存在
// 返回: 是否有字段需要迁移（需要保存配置）
func (cm *ConfigManager) applyConfigDefaults(rawJSON []byte) bool {
	needSave := false

	// FuzzyModeEnabled 默认值处理：
	// 由于 bool 零值是 false，无法区分"用户设为 false"和"字段不存在"
	// 通过检查原始 JSON 是否包含该字段来判断
	var rawMap map[string]json.RawMessage
	if err := json.Unmarshal(rawJSON, &rawMap); err == nil {
		if _, exists := rawMap["fuzzyModeEnabled"]; !exists {
			// 字段不存在，设为默认值 true
			cm.config.FuzzyModeEnabled = true
			needSave = true
			log.Printf("[Config-Migration] FuzzyModeEnabled 字段不存在，设为默认值 true")
		}
		if _, exists := rawMap["stripBillingHeader"]; !exists {
			// 字段不存在，设为默认值 true
			cm.config.StripBillingHeader = true
			needSave = true
			log.Printf("[Config-Migration] StripBillingHeader 字段不存在，设为默认值 true")
		}
	}

	return needSave
}

func (cm *ConfigManager) applyCodexToolCompatMigration(rawJSON []byte) bool {
	var rawMap map[string]json.RawMessage
	if err := json.Unmarshal(rawJSON, &rawMap); err != nil {
		return false
	}
	updated := false
	apply := func(raw json.RawMessage, channels *[]UpstreamConfig, channelName string) {
		var rawChannels []map[string]json.RawMessage
		if err := json.Unmarshal(raw, &rawChannels); err != nil {
			return
		}
		for i := range *channels {
			if i >= len(rawChannels) {
				continue
			}
			rawChannel := rawChannels[i]
			if (*channels)[i].CodexToolCompat != nil {
				continue
			}
			if rawCodexToolsCompat, ok := rawChannel["codexToolsCompat"]; ok {
				var v bool
				if err := json.Unmarshal(rawCodexToolsCompat, &v); err == nil {
					(*channels)[i].CodexToolCompat = &v
					updated = true
					log.Printf("[Config-Migration] %s 渠道 [%d] %s codexToolsCompat 已迁移为 codexToolCompat", channelName, i, (*channels)[i].Name)
				}
				continue
			}
			if rawStrip, ok := rawChannel["stripCodexClientTools"]; ok {
				var v bool
				if err := json.Unmarshal(rawStrip, &v); err == nil && v {
					(*channels)[i].CodexToolCompat = &v
					updated = true
					log.Printf("[Config-Migration] %s 渠道 [%d] %s stripCodexClientTools 已迁移为 codexToolCompat", channelName, i, (*channels)[i].Name)
				}
			}
		}
	}
	if raw, ok := rawMap["upstream"]; ok {
		apply(raw, &cm.config.Upstream, "Messages")
	}
	if raw, ok := rawMap["responsesUpstream"]; ok {
		apply(raw, &cm.config.ResponsesUpstream, "Responses")
	}
	if raw, ok := rawMap["geminiUpstream"]; ok {
		apply(raw, &cm.config.GeminiUpstream, "Gemini")
	}
	if raw, ok := rawMap["chatUpstream"]; ok {
		apply(raw, &cm.config.ChatUpstream, "Chat")
	}
	if raw, ok := rawMap["imagesUpstream"]; ok {
		apply(raw, &cm.config.ImagesUpstream, "Images")
	}
	return updated
}

func (cm *ConfigManager) applyServiceTypeDefaults() bool {
	updated := false

	apply := func(channels []UpstreamConfig, fallback, channelName string) {
		for i := range channels {
			normalized := normalizeUpstreamServiceType(channels[i].ServiceType, fallback)
			if channels[i].ServiceType == normalized {
				continue
			}
			channels[i].ServiceType = normalized
			updated = true
			log.Printf("[Config-Migration] %s 渠道 [%d] %s serviceType 为空，已回填为 %s", channelName, i, channels[i].Name, normalized)
		}
	}

	apply(cm.config.Upstream, "claude", "Messages")
	apply(cm.config.ResponsesUpstream, "responses", "Responses")
	apply(cm.config.GeminiUpstream, "gemini", "Gemini")
	apply(cm.config.ChatUpstream, "openai", "Chat")
	for i := range cm.config.ImagesUpstream {
		normalized, err := normalizeImagesServiceType(cm.config.ImagesUpstream[i].ServiceType)
		if err != nil {
			cm.config.ImagesUpstream[i].ServiceType = "openai"
			updated = true
			log.Printf("[Config-Migration] Images 渠道 [%d] %s serviceType=%s 不受支持，已强制改为 openai", i, cm.config.ImagesUpstream[i].Name, normalizeUpstreamServiceType(cm.config.ImagesUpstream[i].ServiceType, "openai"))
			continue
		}
		if cm.config.ImagesUpstream[i].ServiceType != normalized {
			cm.config.ImagesUpstream[i].ServiceType = normalized
			updated = true
			log.Printf("[Config-Migration] Images 渠道 [%d] %s serviceType 为空，已回填为 %s", i, cm.config.ImagesUpstream[i].Name, normalized)
		}
	}

	return updated
}

// migrateOldFormat 迁移旧格式配置，返回是否有迁移
func (cm *ConfigManager) migrateOldFormat() bool {
	needMigration := false

	// 迁移 Messages 渠道
	if cm.migrateUpstreams(cm.config.Upstream, cm.config.CurrentUpstream, "Messages") {
		needMigration = true
	}

	// 迁移 Responses 渠道
	if cm.migrateUpstreams(cm.config.ResponsesUpstream, cm.config.CurrentResponsesUpstream, "Responses") {
		needMigration = true
	}

	if needMigration {
		log.Printf("[Config-Migration] 检测到旧格式配置，正在迁移到新格式...")
	}

	return needMigration
}

// migrateUpstreams 迁移单个渠道列表
func (cm *ConfigManager) migrateUpstreams(upstreams []UpstreamConfig, currentIdx int, name string) bool {
	if len(upstreams) == 0 {
		return false
	}

	// 检查是否已有 status 字段
	for _, up := range upstreams {
		if up.Status != "" {
			return false
		}
	}

	// 需要迁移
	if currentIdx < 0 || currentIdx >= len(upstreams) {
		currentIdx = 0
	}

	for i := range upstreams {
		if i == currentIdx {
			upstreams[i].Status = "active"
		} else {
			upstreams[i].Status = "disabled"
		}
	}

	log.Printf("[Config-Migration] %s 渠道 [%d] %s 已设置为 active，其他 %d 个渠道已设为 disabled",
		name, currentIdx, upstreams[currentIdx].Name, len(upstreams)-1)

	return true
}

// validateChannelKeys 自检渠道密钥配置
// 没有配置 API key 的渠道，即使状态为 active 也应暂停
// 返回 true 表示有配置被修改，需要保存
func (cm *ConfigManager) validateChannelKeys() bool {
	modified := false

	// 检查 Messages 渠道
	for i := range cm.config.Upstream {
		upstream := &cm.config.Upstream[i]
		status := upstream.Status
		if status == "" {
			status = "active"
		}

		// 如果是 active 状态但没有配置 key，自动设为 suspended
		if status == "active" && len(upstream.APIKeys) == 0 {
			upstream.Status = "suspended"
			modified = true
			log.Printf("[Config-Validate] 警告: Messages 渠道 [%d] %s 没有配置 API key，已自动暂停", i, upstream.Name)
		}
	}

	// 检查 Responses 渠道
	for i := range cm.config.ResponsesUpstream {
		upstream := &cm.config.ResponsesUpstream[i]
		status := upstream.Status
		if status == "" {
			status = "active"
		}

		// 如果是 active 状态但没有配置 key，自动设为 suspended
		if status == "active" && len(upstream.APIKeys) == 0 {
			upstream.Status = "suspended"
			modified = true
			log.Printf("[Config-Validate] 警告: Responses 渠道 [%d] %s 没有配置 API key，已自动暂停", i, upstream.Name)
		}
	}

	// 检查 Chat 渠道
	for i := range cm.config.ChatUpstream {
		upstream := &cm.config.ChatUpstream[i]
		status := upstream.Status
		if status == "" {
			status = "active"
		}

		// 如果是 active 状态但没有配置 key，自动设为 suspended
		if status == "active" && len(upstream.APIKeys) == 0 {
			upstream.Status = "suspended"
			modified = true
			log.Printf("[Config-Validate] 警告: Chat 渠道 [%d] %s 没有配置 API key，已自动暂停", i, upstream.Name)
		}
	}

	// 检查 Gemini 渠道
	for i := range cm.config.GeminiUpstream {
		upstream := &cm.config.GeminiUpstream[i]
		status := upstream.Status
		if status == "" {
			status = "active"
		}

		// 如果是 active 状态但没有配置 key，自动设为 suspended
		if status == "active" && len(upstream.APIKeys) == 0 {
			upstream.Status = "suspended"
			modified = true
			log.Printf("[Config-Validate] 警告: Gemini 渠道 [%d] %s 没有配置 API key，已自动暂停", i, upstream.Name)
		}
	}

	// 检查 Images 渠道
	for i := range cm.config.ImagesUpstream {
		upstream := &cm.config.ImagesUpstream[i]
		status := upstream.Status
		if status == "" {
			status = "active"
		}

		// 如果是 active 状态但没有配置 key，自动设为 suspended
		if status == "active" && len(upstream.APIKeys) == 0 {
			upstream.Status = "suspended"
			modified = true
			log.Printf("[Config-Validate] 警告: Images 渠道 [%d] %s 没有配置 API key，已自动暂停", i, upstream.Name)
		}
	}

	return modified
}

// saveConfigLocked 保存配置（已加锁）
func (cm *ConfigManager) saveConfigLocked(config Config) error {
	// 备份当前配置
	cm.backupConfig()

	// 清理已废弃字段，确保不会被序列化到 JSON
	config.CurrentUpstream = 0
	config.CurrentResponsesUpstream = 0

	data, err := json.MarshalIndent(config, "", "  ")
	if err != nil {
		return err
	}

	cm.config = config
	return os.WriteFile(cm.configFile, data, 0600) // 仅所有者可读写，保护敏感配置
}

// SaveConfig 保存配置
func (cm *ConfigManager) SaveConfig() error {
	cm.mu.Lock()
	defer cm.mu.Unlock()
	return cm.saveConfigLocked(cm.config)
}

// backupConfig 备份配置
func (cm *ConfigManager) backupConfig() {
	if _, err := os.Stat(cm.configFile); os.IsNotExist(err) {
		return
	}

	backupDir := filepath.Join(filepath.Dir(cm.configFile), "backups")
	if err := os.MkdirAll(backupDir, 0700); err != nil { // 仅所有者可访问
		log.Printf("[Config-Backup] 警告: 创建备份目录失败: %v", err)
		return
	}

	// 读取当前配置
	data, err := os.ReadFile(cm.configFile)
	if err != nil {
		log.Printf("[Config-Backup] 警告: 读取配置文件失败: %v", err)
		return
	}

	// 创建备份文件
	timestamp := time.Now().Format("2006-01-02T15-04-05")
	backupFile := filepath.Join(backupDir, fmt.Sprintf("config-%s.json", timestamp))
	if err := os.WriteFile(backupFile, data, 0600); err != nil { // 仅所有者可读写
		log.Printf("[Config-Backup] 警告: 写入备份文件失败: %v", err)
		return
	}

	// 清理旧备份
	cm.cleanupOldBackups(backupDir)
}

// cleanupOldBackups 清理旧备份
func (cm *ConfigManager) cleanupOldBackups(backupDir string) {
	entries, err := os.ReadDir(backupDir)
	if err != nil {
		return
	}

	if len(entries) <= maxBackups {
		return
	}

	// 删除最旧的备份
	for i := 0; i < len(entries)-maxBackups; i++ {
		os.Remove(filepath.Join(backupDir, entries[i].Name()))
	}
}

// startWatcher 启动文件监听
func (cm *ConfigManager) startWatcher() error {
	watcher, err := fsnotify.NewWatcher()
	if err != nil {
		return err
	}

	cm.watcher = watcher

	go func() {
		for {
			select {
			case <-cm.stopChan:
				return
			case event, ok := <-watcher.Events:
				if !ok {
					return
				}
				if event.Op&fsnotify.Write == fsnotify.Write {
					log.Printf("[Config-Watcher] 检测到配置文件变化，重载配置...")
					if err := cm.loadConfig(); err != nil {
						log.Printf("[Config-Watcher] 警告: 配置重载失败: %v", err)
					} else {
						log.Printf("[Config-Watcher] 配置已重载")
					}
				}
			case err, ok := <-watcher.Errors:
				if !ok {
					return
				}
				log.Printf("[Config-Watcher] 警告: 文件监听错误: %v", err)
			}
		}
	}()

	return watcher.Add(cm.configFile)
}

// Close 关闭 ConfigManager 并释放资源（幂等，可安全多次调用）
func (cm *ConfigManager) Close() error {
	var closeErr error
	cm.closeOnce.Do(func() {
		// 通知所有 goroutine 停止
		if cm.stopChan != nil {
			close(cm.stopChan)
		}

		// 关闭文件监听器
		if cm.watcher != nil {
			closeErr = cm.watcher.Close()
		}
	})
	return closeErr
}

// deepCopy 创建配置的深拷贝
func (c Config) deepCopy() Config {
	data, err := json.Marshal(c)
	if err != nil {
		return c
	}
	var copy Config
	if err := json.Unmarshal(data, &copy); err != nil {
		return c
	}
	return copy
}

// hasConfigChanged 检测配置是否发生了实质性变化
func (cm *ConfigManager) hasConfigChanged(old, new Config) bool {
	// 清理废弃字段以确保比较准确
	old.CurrentUpstream = 0
	old.CurrentResponsesUpstream = 0
	new.CurrentUpstream = 0
	new.CurrentResponsesUpstream = 0

	oldData, err := json.Marshal(old)
	if err != nil {
		return true
	}
	newData, err := json.Marshal(new)
	if err != nil {
		return true
	}
	return !bytes.Equal(oldData, newData)
}

# CodeBuddy API 页面样式指南

本文档是 `CodebuddyApiServicePage` 重构的样式参考蓝本，源自 `CodexApiServicePage.css` + `base.css` 的双主题变量体系，供后续 Tab/Modal/账号页对接时对照使用。

## 1. 命名空间映射

CodeBuddy API 页面沿用 Codex API 的视觉语言，但使用独立命名空间 `cb-api-*` 避免选择器冲突：

| Codex 命名 | CodeBuddy 命名 | 说明 |
| --- | --- | --- |
| `.codex-api-service-page` | `.cb-api-service-page` | 页面根容器 |
| `.codex-api-service-hero` | `.cb-api-service-hero` | Hero 头部 |
| `.codex-api-service-content` | `.cb-api-service-content` | 内容区 |
| `.codex-api-service-title-row` | `.cb-api-service-title-row` | 标题行 |
| `.codex-api-service-status.running` | `.cb-api-service-status.running` | 运行中徽章 |
| `.codex-api-service-status.stopped` | `.cb-api-service-status.stopped` | 已停用徽章 |
| `.codex-api-service-current-tag` | `.cb-api-service-current-tag` | 当前标签 |
| `.codex-api-service-pill.mode-sidecar` | `.cb-api-service-pill.mode-sidecar` | sidecar 模式 |
| `.codex-api-service-pill.mode-legacy` | `.cb-api-service-pill.mode-legacy` | legacy 模式 |
| `.codex-api-service-hero-actions` | `.cb-api-service-hero-actions` | Hero 操作按钮组 |
| `.codex-api-tabs` | `.cb-api-tabs` | Tab 胶囊导航 |
| `.codex-api-tab` | `.cb-api-tab` | 单个 Tab |
| `.codex-api-stat-card` | `.cb-api-stat-card` | 统计卡片 |
| `.codex-api-config-panel` | `.cb-api-config-panel` | 配置面板 |
| `.codex-api-health-panel` | `.cb-api-health-panel` | 健康面板 |
| `.codex-api-protocol-card` | `.cb-api-protocol-card` | 协议兼容卡片 |
| `.codex-api-log-row` | `.cb-api-log-row` | 日志行 |
| `.codex-api-modal-backdrop` | `.cb-api-modal-backdrop` | Modal 背景遮罩 |

## 2. CSS 变量定义（必须放在页面根容器内）

```css
.cb-api-service-page {
  /* 玻璃拟态表面 */
  --cb-api-surface: color-mix(in srgb, var(--bg-card) 92%, rgba(255, 255, 255, 0.54));
  --cb-api-surface-strong: color-mix(in srgb, var(--bg-card) 96%, rgba(255, 255, 255, 0.74));
  --cb-api-surface-soft: color-mix(in srgb, var(--primary) 4%, rgba(255, 255, 255, 0.72));

  /* 边框 */
  --cb-api-border: color-mix(in srgb, var(--border) 78%, rgba(148, 163, 184, 0.14));
  --cb-api-border-soft: color-mix(in srgb, var(--border-light) 82%, rgba(148, 163, 184, 0.12));

  /* 阴影 */
  --cb-api-shadow: 0 10px 24px -22px rgba(15, 23, 42, 0.42);
  --cb-api-shadow-hover: 0 14px 28px -24px rgba(15, 23, 42, 0.48);

  display: flex;
  flex-direction: column;
  width: 100%;
  min-width: 0;
  min-height: 100%;
  gap: 12px;
  animation: fadeUp 0.5s ease;
}
```

## 3. 暗色模式专属覆盖（必须）

参考 `pages/codex.css` 第 93 行 `[data-theme="dark"] .codex-overview-filter-banner.is-active` 的覆盖写法：

```css
/* 暗色模式：状态色硬编码 → 改用语义亮色 */
[data-theme="dark"] .cb-api-service-status.running,
[data-theme="dark"] .cb-api-service-pill.success {
  color: #4ade80;
}

[data-theme="dark"] .cb-api-service-status.stopped,
[data-theme="dark"] .cb-api-service-message.warning {
  color: #fbbf24;
}

[data-theme="dark"] .cb-api-service-pill.mode-sidecar {
  color: #5eead4;
}

[data-theme="dark"] .cb-api-service-pill.mode-legacy {
  color: #a5b4fc;
}

[data-theme="dark"] .cb-api-service-pill.error {
  color: #fca5a5;
}

/* 暗色模式：阴影加深 25% */
[data-theme="dark"] .cb-api-service-page {
  --cb-api-shadow: 0 12px 28px -22px rgba(0, 0, 0, 0.55);
  --cb-api-shadow-hover: 0 16px 32px -24px rgba(0, 0, 0, 0.62);
}

/* 暗色模式：玻璃拟态更透明 */
[data-theme="dark"] .cb-api-service-hero {
  background:
    linear-gradient(135deg, color-mix(in srgb, var(--primary) 12%, transparent), transparent 42%),
    color-mix(in srgb, var(--bg-card) 88%, rgba(255, 255, 255, 0.06));
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
}

/* 暗色模式：Modal 遮罩更黑 */
[data-theme="dark"] .cb-api-modal-backdrop {
  background: rgba(0, 0, 0, 0.62);
}

/* 暗色模式：日志行 hover 加深 */
[data-theme="dark"] .cb-api-log-row:hover {
  background: color-mix(in srgb, var(--primary) 6%, transparent);
}
```

## 4. Hero 区结构（必须对齐 Codex）

```tsx
<section className="cb-api-service-hero">
  <div className="cb-api-service-hero-main">
    <div className="cb-api-service-title-row">
      <span className="cb-api-service-title-icon">
        <CodebuddyIcon />
      </span>
      <div className="cb-api-service-title-copy">
        <div className="cb-api-service-title-line">
          <h1>{t('codebuddy.apiService.title', 'CodeBuddy API')}</h1>
          <span className="cb-api-service-current-tag">
            {t('codebuddy.apiService.current', '当前')}
          </span>
          <span className={`cb-api-service-status ${state.running ? 'running' : 'stopped'}`}>
            {state.running
              ? t('codebuddy.apiService.statusRunning', '运行中')
              : t('codebuddy.apiService.statusStopped', '已停用')}
          </span>
          <span className="cb-api-service-pill mode-sidecar">
            {t('codebuddy.apiService.modeSidecar', '反代模式')}
          </span>
        </div>
      </div>
    </div>
  </div>
  <div className="cb-api-service-hero-actions">
    <button className="btn btn-secondary" onClick={handleRefresh}>
      <RefreshCw className={loading ? 'spinning' : ''} />
    </button>
    <button
      className={`btn ${state.running ? 'btn-danger' : 'btn-primary'}`}
      onClick={() => toggleRunning()}
    >
      {state.running ? '停止' : '启动'}
    </button>
  </div>
</section>
```

## 5. 5 个 Tab 胶囊导航（必须对齐 Codex）

```tsx
<nav className="cb-api-tabs">
  {([
    { id: 'overview', label: '服务总览' },
    { id: 'keys', label: '客户端 Key' },
    { id: 'accounts', label: '账号池' },
    { id: 'models', label: '模型与能力' },
    { id: 'logs', label: '统计与日志' },
  ] as const).map((tab) => (
    <button
      key={tab.id}
      className={`cb-api-tab ${activeTab === tab.id ? 'is-active' : ''}`}
      onClick={() => setActiveTab(tab.id)}
    >
      {tab.label}
    </button>
  ))}
</nav>
```

## 6. 统计卡片（5 列，必须对齐 Codex）

```tsx
<div className="cb-api-stat-grid">
  <StatCard label="总请求数" value={stats.totalRequests} icon={<Activity />} />
  <StatCard label="图片请求" value={stats.imageRequests} icon={<ImageIcon />} />
  <StatCard label="总 Token" value={stats.totalTokens} icon={<Hash />} />
  <StatCard label="估算价值" value={`$${stats.estimatedValue}`} icon={<DollarSign />} />
  <StatCard label="平均延迟" value={`${stats.avgLatency}ms`} icon={<Timer />} />
</div>
```

## 7. 协议兼容卡片（5 张，必须对齐 Codex）

每张卡片含：
- 协议图标（OpenAI / Responses / Anthropic / Gemini / Ollama）
- 协议名称
- 环境变量片段（可复制）
- 启用状态

```tsx
<div className="cb-api-protocol-grid">
  <ProtocolCard
    name="OpenAI Chat"
    envVar="OPENAI_BASE_URL"
    baseUrl={state.baseUrl}
    color="success"
  />
  <ProtocolCard
    name="OpenAI Responses"
    envVar="OPENAI_BASE_URL"
    baseUrl={`${state.baseUrl}/v1/responses`}
    color="primary"
  />
  <ProtocolCard
    name="Anthropic Messages"
    envVar="ANTHROPIC_BASE_URL"
    baseUrl={`${state.baseUrl}/v1/messages`}
    color="warning"
  />
  <ProtocolCard
    name="Gemini"
    envVar="GEMINI_BASE_URL"
    baseUrl={`${state.baseUrl}/v1beta`}
    color="info"
  />
  <ProtocolCard
    name="Ollama Bridge"
    envVar="OLLAMA_BASE_URL"
    baseUrl={`${state.baseUrl}/api`}
    color="muted"
  />
</div>
```

## 8. 刷新按钮统一规格

每个 Tab 顶部右上角必须有刷新按钮：

```tsx
<header className="cb-api-section-header">
  <h2>{t(`codebuddy.apiService.tab.${tabId}`)}</h2>
  <button
    className="cb-api-refresh-btn"
    onClick={handleRefresh}
    disabled={loading}
    title={t('common.refresh', '刷新')}
  >
    <RefreshCw className={loading ? 'cb-api-spinning' : ''} size={14} />
  </button>
</header>
```

```css
.cb-api-refresh-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: 1px solid var(--cb-api-border);
  border-radius: 8px;
  background: var(--cb-api-surface);
  color: var(--text-secondary);
  cursor: pointer;
  transition: all 0.2s ease;
}

.cb-api-refresh-btn:hover {
  background: var(--cb-api-surface-strong);
  color: var(--primary);
  border-color: color-mix(in srgb, var(--primary) 24%, transparent);
}

.cb-api-refresh-btn:disabled {
  cursor: not-allowed;
  opacity: 0.6;
}

.cb-api-spinning {
  animation: cb-api-spin 0.8s linear infinite;
}

@keyframes cb-api-spin {
  to { transform: rotate(360deg); }
}
```

## 9. 局域网 URL 显示（任务 6 专属）

```tsx
{state.scope === 'lan' && state.lanBaseUrl && (
  <div className="cb-api-lan-url-row">
    <span className="cb-api-lan-url-label">
      {t('codebuddy.apiService.lanUrl', '局域网 URL')}
    </span>
    <code className="cb-api-lan-url-value">{state.lanBaseUrl}</code>
    <button className="cb-api-copy-btn" onClick={() => copyToClipboard(state.lanBaseUrl)}>
      <Copy size={13} />
    </button>
    <span className="cb-api-lan-warning">
      {t('codebuddy.apiService.lanWarning', '局域网模式：请确保网络可信')}
    </span>
  </div>
)}
```

## 10. 6 套主题色变体验证清单

实现完成后必须逐套验证（每套 × 双主题 = 12 种组合）：

- [ ] `default`（紫蓝） - 浅色 / 暗色
- [ ] `nord`（冷蓝绿） - 浅色 / 暗色
- [ ] `tokyo-night`（深紫蓝） - 浅色 / 暗色
- [ ] `catppuccin`（柔紫粉） - 浅色 / 暗色
- [ ] `gruvbox`（暖橙绿） - 浅色 / 暗色
- [ ] `everforest`（柔绿） - 浅色 / 暗色

**关键校验点**：
1. Hero 渐变背景在所有主题色下都保持 7% 透明度不破坏可读性
2. 状态徽章颜色（绿/黄/红/紫/青）在所有主题色 + 暗色下都满足 WCAG AA 4.5:1 对比度
3. Tab 激活态的 `--primary` 颜色在 6 套主题色下都明确可辨
4. 协议兼容卡片的 5 种 brand 色不与主题色冲突
5. 玻璃拟态背景在暗色下保留质感（rgba(255,255,255,...) 混入）

## 11. 严禁清单

- ❌ `background: white` / `background: #fff`
- ❌ `color: black` / `color: #000`
- ❌ `background: #f5f5f5` 等无视主题的硬编码
- ❌ 直接复制 Codex 的 `#15803d / #b45309 / #4338ca / #0f766e` 而不加暗色覆盖
- ❌ 使用 `rgba(34, 197, 94, 0.12)` 等硬编码半透明（应用 `color-mix(in srgb, var(--success) 12%, transparent)`）
- ❌ 用 `div` 模拟 `button` / `input`
- ❌ 文件超过 300 行不拆分

## 12. 文件结构建议

```
src/pages/CodebuddyApiServicePage.tsx           # 主页面（Tab 路由）
src/pages/CodebuddyApiServicePage.css           # 主样式
src/pages/codebuddy-api/
  CodebuddyApiServiceSharedView.tsx             # 可复用视图组件
  OverviewTab.tsx                                # 服务总览
  KeysTab.tsx                                    # 客户端 Key
  AccountsTab.tsx                                # 账号池
  ModelsTab.tsx                                  # 模型与能力
  LogsTab.tsx                                    # 统计与日志
  StatCard.tsx                                   # 统计卡片
  ProtocolCard.tsx                               # 协议兼容卡片
  RefreshButton.tsx                              # 通用刷新按钮
  cb-api-shared.css                              # 共享样式
src/components/CodebuddyLocalAccessModal.tsx    # 便捷启动 API Modal
src/components/CodebuddyLocalAccessModal.css    # Modal 样式
```

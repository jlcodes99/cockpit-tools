# WorkBuddy Codex 操作按钮图标化设计

## 意图
将 WorkBuddy 账号卡片中的“接入 Codex”文字按钮改为与其他卡片操作一致的纯图标按钮，减少操作栏宽度差异并提升视觉统一性。

## 范围
- 保留现有 `addCodexAccountFromWorkbuddy` 调用、禁用态、加载态和绑定行为不变。
- 复用现有 `card-action-btn` 样式和操作栏布局。
- 使用项目已有的连接/链接类图标表达“接入 Codex”。
- 保留 `title="接入 Codex"`，并补充 `aria-label="接入 Codex"`，确保鼠标悬停提示和无障碍可用。

## 不包含
- 不新增 Codex 品牌图标资源。
- 不修改 WorkBuddy/Codex provider gateway、账号绑定协议或后端命令。
- 不调整其他账号卡片操作按钮。

## 实现方案
在 `WorkbuddyAccountsPage` 的账号卡片操作扩展按钮中，将文字内容替换为现有图标组件；按钮仍使用 `card-action-btn`，加载时保留“接入中”状态图标或现有加载表现。完成后运行针对性的前端检查，并通过 GitHub Edge 会话提交独立 PR。

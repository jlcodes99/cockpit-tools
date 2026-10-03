# Antigravity CLI（agy）账号管理

Antigravity CLI 是 Antigravity 平台分组下的子平台（与 Antigravity、Antigravity IDE 并列），通过侧边栏 Antigravity 分组或页面左上角的平台切换器进入，路由为 `antigravity-cli`。页面结构与其他平台一致：顶部为平台切换器与 `agy` 版本徽章，其下为可折叠的管理说明、CLI 登录状态条和账号列表。

- **导入现有登录**：状态条显示「已登录 · 未导入」时点击「导入 CLI 账号」，或在 添加账号 → 导入 → 导入 CLI 账号。从系统钥匙环读取，刷新 OAuth Token 并保存到 Cockpit 现有账号库，复用账号分组、备注、导入导出和配额功能。
- **添加账号**：使用现有 OAuth 授权流程。添加后点击账号卡片的切换按钮，将该账号写入 CLI 使用的系统凭据。
- **切换账号**：先退出已有 `agy` 会话，切换后重新启动。Cockpit 不会关闭终端、不修改 IDE 的 `state.vscdb` 或默认实例，也不触发 IDE 无感切号。
- **打开 CLI**：自动检测 PATH 和官方默认安装目录中的 `agy`，在右上角显示版本；状态条中的「打开 CLI」弹出启动对话框，按设置中的默认终端启动。可选择工作目录（会记住上次的选择），留空使用用户主目录。未检测到 `agy` 时显示安装指南入口。
- **检查状态**：以钥匙环中的实际凭据匹配账号，不使用 Cockpit 的全局当前账号作为回退。已登录、未导入、未登录、API Key 模式、钥匙环异常分别显示；回到窗口时重新检查。
- **登出**：在 `agy` 中执行 `/logout`，随后点击检查状态。删除 Cockpit 中的账号只删除其管理记录，不等于 CLI 登出。

## 凭据与平台

| 平台 | 读取与写入方式 |
| --- | --- |
| Linux | Secret Service，通过 `secret-tool`，属性 `service=gemini`、`username=antigravity` |
| macOS | Keychain，通过 `security`，service `gemini`、account `antigravity`；兼容 `go-keyring-base64:` 编码 |
| Windows | Credential Manager，Generic Credential `gemini:antigravity` |

Linux 需要 `secret-tool`（Debian/Ubuntu 的 `libsecret-tools`）以及可访问、已解锁的用户钥匙环。不要直接修改 `login.keyring`。

CLI 与新版 Antigravity 桌面版共用系统凭据，因此切换也会影响桌面版的新会话。系统钥匙环没有独立实例命名空间，此入口不提供彼此隔离的多账号并行会话。IDE 的账号切换保持独立。

`~/.gemini/antigravity-cli/` 用于配置、会话及缓存。Google 账号切换不会覆盖这些文件。如果 `settings.json` 中设置了 `"modelProvider": "gemini"`，CLI 使用 API Key 而非 Google 登录，Cockpit 会阻止误报切号成功；先移除此配置并重启 CLI，再切换 Google 账号。当前账号导入支持 `consumer` OAuth 凭据。

## 实现与验证

- `modules/antigravity_cli.rs`：安装检测、登录状态、本地导入、刷新后写入并核对系统凭据。
- `modules/antigravity_credential.rs`：跨平台凭据格式；保留 OAuth `id_token`，写入失败不会先删除原凭据，解析错误不输出原始 Token。
- `commands/antigravity_cli.rs`：状态与终端启动命令，工作目录和执行路径按 shell 规则引用。
- 前端：`PlatformId` 新增 `antigravity_cli`（页面 `antigravity-cli`），默认归入 `antigravity-suite` 分组；已有布局通过 `antigravityCliSuiteMigrated` 一次性迁移加入该分组。账号页复用 `AccountsPage`，以 `platform="antigravity_cli"` 固定账号目标；CLI 专属界面位于 `components/antigravity-cli/`，状态读取在 `hooks/useAntigravityCliStatus.ts`。
- 账号 API 使用 `runtimeTarget: "antigravity_cli"`；OAuth、配额和存储沿用 Antigravity 账号体系，因此仪表盘计数、账号导出不会重复统计 CLI。UI 中的 CLI 当前账号单独保存，避免影响 IDE 的当前账号标记。

常规验证：`npm test`、`npm run build`、`cargo check -p cockpit-tools --lib --locked`、`cargo test -p cockpit-tools --lib antigravity -- --test-threads=1`。

手工验收：在专用测试账号下导入钥匙环登录，切换两个 OAuth 账号并重启 `agy` 核对身份；检查 IDE 未被重启；执行 `/logout` 后核对当前标记清空；分别验证钥匙环锁定、CLI 未安装、API Key 模式和带空格/引号的工作目录。macOS 和 Windows 需要在对应平台验收。

参考：[官方安装和认证文档](https://www.antigravity.google/docs/cli/install/)。本机对照版本：`agy 1.2.16`。

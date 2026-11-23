# Release Process

本文档描述仓库当前由 `.github/workflows/release.yml` 执行的发布流程。发布行为以 workflow 和 `scripts/release/` 下的脚本为准；如果两者与本文不一致，应先修正文档或 workflow，再发版。

## 共享开发工作区

- **固定入口。** 本机开发、修复和发版统一使用 `/private/var/www/antigravity-cockpit-tools` 的本地 `main`。每个会话首次写入或发版前，用 `git rev-parse --show-toplevel` 和 `git branch --show-current` 核对。工作区或分支变化后，在下一次写入前重新核对。切到其他开发分支、新建或使用其他工作树，仅在用户明确要求时执行。
- **旧分支与现有改动。** 需要回到默认入口时，先用 `git status --short --branch` 核对现有改动。能保留这些改动并安全切回时，回到 `main` 后继续写入。切换有冲突时保留现场，说明冲突文件和阻塞原因，解决后再在 `main` 写入。stash 或重置须有用户明确授权，其他会话的改动必须保留。迁移时对照当前 `main`，按功能去重收拢原目录、其他工作树和归档中尚未合入的改动。
- **并行写入。** 不同会话可以并行改互不重叠的范围。同一文件、共享逻辑或中英文更新日志有并行改动时，先核对已有内容，再错开写入，保留他人改动。暂存和提交只选本次授权的范围。
- **开发交接。** 已完成的用户可感知改动写进共用的中英文待发日志，不另建发布清单。交接时记录改动所在的工作区和分支，以及改动在工作区未提交、已提交或仅保存在 stash 或 Codex 工作树快照中的状态。已提交的改动记录提交哈希，stash 或快照记录对象哈希、基线提交和功能范围。未完成和实验改动注明文件和进度。日常开发留在本地，不自动提交或推送。

## 1. 发布前检查

标准发版在仓库根目录执行：

```bash
npm run release:preflight
```

当前 preflight 依次执行：

1. `node scripts/check_locales.cjs`
2. `node scripts/check_modal_sizing.cjs --strict`
3. `npm run typecheck`
4. `npm run build`
5. `cargo check`（`src-tauri`）
6. `cargo test --lib`（`src-tauri`，`RUST_TEST_THREADS=1`）

排障时可以跳过单项：

```bash
node scripts/release/preflight.cjs \
  --skip-locales \
  --skip-typecheck \
  --skip-build \
  --skip-cargo \
  --skip-cargo-test
```

标准发布不得使用 `--skip-*`。明确指定「快速发布」时不运行预检脚本，也不在正式构建前另跑翻译、弹框布局、类型检查、`cargo check` 或测试。普通分支 CI 仍独立运行，结果不阻塞正式 tag 构建。

## 2. 版本与标签

`package.json.version` 是发布 workflow 读取的版本。创建发布标签前先执行：

```bash
npm run sync-version
```

然后确认版本同步后的文件与 changelog 已提交。发布标签必须严格匹配：

```text
v<package.json.version>
```

例如 `package.json.version` 为 `1.3.40` 时，标签必须是 `v1.3.40`。workflow 会在版本或标签不一致时直接失败。

## 3. GitHub Actions 发布目标

当前 release workflow 构建并上传：

- Windows x64（MSI、NSIS 和便携包）
- Windows ARM64（NSIS）
- macOS Apple Silicon (`aarch64`)
- macOS Intel (`x86_64`)
- macOS Universal
- Linux `x86_64`
- Linux `aarch64`

Linux release 同时包含 AppImage、deb 和 rpm，以及可在无桌面环境运行的 x86_64 和 aarch64 独立 CLI 压缩包。独立 CLI 计入发布校验和。macOS Universal DMG 还会用于后续 Homebrew Cask 更新。

macOS 正式构建和 `warm-macos-release-cache.yml` 共用独立的依赖缓存，固定在同一 Rust 版本，并分别按 ARM 和 Intel 编译目标保存。预热在 `main` 的构建配置或依赖变更时后台运行，也可在 `main` 手动启动；用 Tauri 的正式编译参数，只编译，不签名、打包或发布。正式 tag 仅读取缓存，不等待预热；缓存缺失时直接编译。普通 PR CI 的 `ci` profile 缓存保持独立；Windows/Linux 继续使用原有缓存。

新推送不会取消正在编译的预热任务。同一版的 `main` 和 tag 紧接着推时，正式构建可能在缓存写入前就已经开始。需要缩短打 tag 后的等待时，可在准备版本期间让 `main` 预热先完成；这不是发版门槛。下一次发布在 macOS ARM 和 Intel 的任务摘要看 `Exact match`，再对照 Rust 编译耗时；`false` 只表示没有精确匹配，是否恢复了部分缓存以步骤日志为准。

Universal 不再从源码重编两套宿主。ARM 和 Intel release job 在本轮构建后通过 `scripts/release/macos-universal.cjs --mode stage` 上传宿主薄二进制，附带版本、commit、workflow run ID 和 SHA-256。Universal job 使用同一 run 的具名 artifact；`--mode merge` 校验身份、文件哈希和实际架构，合并宿主并逐片比对原始内容，再用 `tauri bundle` 打包。缺失或不匹配直接失败，不回退到其他版本或旧产物。

正式 workflow 只生成一次前端 `dist`，各平台使用同一份 artifact；`tauri.prebuilt-frontend.conf.json` 只覆盖构建钩子，不替换正式 Tauri 配置或更新地址。

ARM 和 Intel 用 `tauri build --bundles app`、Universal 用 `tauri bundle --bundles app`，保留已签名公证的 `.app` 及 Tauri 更新包。随后三个 job 都直接用 `scripts/release/inject-dmg-readme.cjs` 从 `.app` 生成最终 DMG，不先制作另一份镜像。安装说明、Finder 布局、卷图标和文件命名保持不变；仍在安装包生成后签名并公证最终镜像。

macOS 构建使用 `npm run tauri -- build ...` 或 `node scripts/tauri.cjs build ...`（正式 workflow 同样使用后者），在构建结束或失败时清理本轮新增的 Tauri `bundle/macos/rw.*.dmg` 临时挂载，不碰构建前已有的卷及最终 DMG。直接使用 `npx tauri build` 会绕过这层清理。

DMG 重建与验证共用 `scripts/release/dmg-mounts.cjs` 管理本次创建的挂载。正常完成、验证失败及可处理的 `SIGINT` / `SIGTERM` 退出都会尝试卸载；仅卸载本次挂载，不强制推出用户磁盘。卸载失败时打包必须失败，并保留镜像和临时目录供排查，禁止带着挂载删除临时文件。系统强制结束进程（如 `SIGKILL`）或断电无法保证执行清理。

本地测试包统一使用以下入口打包已构建的 `.app`，不需要另写临时挂载脚本：

```bash
node scripts/release/package-local-dmg.cjs \
  --app "target/release/bundle/macos/Cockpit Tools.app" \
  --output ".tmp/local-packages/Cockpit-Tools_<version>_arm64_test.dmg"
```

该入口保留原 App 签名，校验镜像、挂载后逐文件比较内容和符号链接，并验证 App 签名；全部通过且卸载完成后才替换输出文件并写入 `.sha256`。`--stage <directory>` 可替代 `--app`，用于复用已核验的完整暂存目录。脚本不会代替宿主构建、签名或 Apple 公证，也不会自动推出用户安装时另行打开的 DMG。

Tauri release build 使用仓库配置的 updater signing secrets：

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

这两个密钥仅用于 Tauri updater artifact 签名，与 Apple Developer ID 签名不同，必须继续保留。Windows Authenticode 不在本次流程范围内。

### macOS Developer ID 与 Apple 公证

三个 macOS 正式 release job 均要求以下 GitHub Actions repository secrets（缺失即失败，不降级为未签名包）：

| Secret | 内容 |
| --- | --- |
| `APPLE_CERTIFICATE` | 包含私钥的 Developer ID Application `.p12` 文件的 Base64 |
| `APPLE_CERTIFICATE_PASSWORD` | `.p12` 导出密码 |
| `APPLE_SIGNING_IDENTITY` | 完整 `Developer ID Application: … (TEAMID)` 证书名 |
| `APPLE_ID` | 公证用 Apple 账号邮箱 |
| `APPLE_PASSWORD` | Apple App 专用密码，不是账号登录密码 |
| `APPLE_TEAM_ID` | 与签名证书一致的 10 位团队 ID |

`macos-signing.cjs prepare` 在 runner 临时目录创建独立钥匙串、导入证书并验证公证凭据。构建步骤通过 Tauri 完成应用签名、公证和票据附加，开启 hardened runtime；不默认放宽 entitlement。自动更新包继续由 Tauri 生成与签名。

随后直接生成包含安装说明的 DMG，再由 `macos-signing.cjs finalize` 签名并提交最终 DMG 公证，等待上限 30 分钟。校验覆盖原应用、解压后的自动更新应用和最终 DMG 内的应用，要求正确 Developer ID 团队、hardened runtime、有效公证票据及 Gatekeeper 认可；最终 DMG 本身也需通过签名、票据和 Gatekeeper 检查。全部通过后才进入资产暂存、上传与校验和生成。Universal 合并后仍由 Tauri 重新签名，不能复用合并前的签名。

每个 macOS job 上限 120 分钟，结束时 `if: always()` 恢复钥匙串搜索列表并删除本轮临时钥匙串；导入后的 `.p12` 临时文件立即删除。Apple 返回拒绝、超时或凭据错误时停止发布，不移除隔离属性绕过校验。首次公证可能等待较久，超时应检查 Apple submission 状态后重试，不能当作已通过。

本流程不会补签已经发布的历史安装包。只有实际完成签名与公证的产物才能标注为已验证。证书、私钥、专用密码均不得提交到仓库或写入日志。

## 4. Release assets 与 updater manifests

Windows、macOS Apple Silicon/Intel 和 Linux 构建 job 会先通过 `scripts/release/stage_release_assets.cjs` 规范化允许上传的 release assets，再用 `scripts/release/build_target_latest_json.cjs` 生成各 updater target 的 manifest。macOS Universal job 只规范化并上传 Universal DMG，不生成 updater manifest；该 DMG 供后续 Homebrew Cask 更新使用。

所有平台完成后，`finalize-legacy-latest` job 会下载 release assets，并用：

```bash
node scripts/release/build_merged_latest_json.cjs
```

生成完整 legacy `latest.json`。workflow 会检查其中至少包含：

- `darwin-aarch64`
- `darwin-x86_64`
- `windows-x86_64`
- `windows-x86_64-nsis`
- Linux x86_64 / aarch64 的 AppImage、deb、rpm targets

随后会通过 `scripts/release/verify_published_updater_manifests.cjs` 对公开 updater 状态做端到端验证。

## 5. SHA256SUMS

`finalize-legacy-latest` job 复用同一次下载的 release assets，在生成完整 `latest.json` 后计算 SHA-256，并一并上传两个文件：

```text
SHA256SUMS.txt
```

不要依赖文档中不存在的 `npm run release:checksums` 命令。当前 checksum 的权威实现位于 release workflow 本身；`scripts/release/gen_checksums.cjs` 是可单独调用的脚本，但不是 `package.json` 中的标准 npm script。

## 6. Homebrew Cask

release workflow 会下载已发布的 Universal DMG，计算 SHA-256，然后更新：

```text
Casks/cockpit-tools.rb
```

该更新通过自动创建的 PR 提交，而不是本地 `npm run release:github-and-cask`。当前 `package.json` 没有这个 npm script，因此不要按旧文档中的本地一键脚本操作。

## 7. 发版顺序

0. **确定范围并收齐**：未指定只发某项时，「发版」和「快速发版」都包含本项目所有已完成、尚未公开发布的改动，不限于当前会话。以最近一个仍公开的正式版为基线，对照本地 `main` 的提交、未提交改动和待发日志；如仍有分散开发的遗留改动，由发版会话核对对应分支、工作区和归档快照，只合入尚未包含的最终实现。只有同一功能存在冲突且无法从代码和历史判断取舍时，才询问具体冲突。
1. 更新 `package.json` 版本。
2. 按最近公开正式版到本次准备发布的实际代码差异，更新 `CHANGELOG.md` 与 `CHANGELOG.zh-CN.md`，不把未合入内容写成已发布。
3. 执行 `npm run sync-version`。标准发布按 `AGENTS.md` 执行本地检查；明确指定快速发布时跳过这些检查，不等待普通 CI 或缓存预热。
4. 在共享的本地 `main` 提交已收齐的改动。打 tag 前对照最终提交和待发日志，告知用户本次包含什么、哪些已知改动未纳入及原因。范围明确时无需再次审批。
5. 从核对过的 `main` 提交创建 `v<version>` 标签，先推送 `main`，再推送标签。
6. 分支和 tag 到达远端即报告「已触发构建」。只有正式 Release 和 `latest.json` 可用时才报告「用户可更新」。Homebrew Cask 在发布后异步更新。

快速发布不强制清理本地 Rust/Tauri 编译产物。仅讨论发布、修改规则或验证代码不授权提交、推送或触发发布。

## 8. 草稿发布与公开校验

当前 workflow 先创建 draft Release。全部平台构建、完整 manifests、checksums 和草稿资产校验完成后，`publish-release` 才公开 Release 并设置为 latest，随后检查公共下载地址；公开发布后不再补齐缺失平台来绕过校验。

GitHub 公共 latest 路由可能晚于 Release 状态变更。如果公开校验仍返回旧版本，应核对 Release API、公共下载重定向与本轮清单内容，保留版本、签名和资产归属校验；确认传播完成后可单独重跑发布校验 job。

同版本撤回重发必须经用户明确授权，先核对并清理旧同版本 Release 和标签，再从新提交重建标签。发布后应额外比较公开清单与新 Release 资产的内容哈希，避免同版本旧缓存仅凭版本号通过检查。已安装同版本的用户需要重新下载安装，不会因同版本号重发而自动升级。

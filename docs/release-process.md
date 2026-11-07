# Release Process

本文档描述仓库当前由 `.github/workflows/release.yml` 执行的发布流程。发布行为以 workflow 和 `scripts/release/` 下的脚本为准；如果两者与本文不一致，应先修正文档或 workflow，再发版。

## 1. 发布前检查

在仓库根目录执行：

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

正式发布不应为了绕过失败而随意使用 skip 参数。

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

- Windows
- macOS Apple Silicon (`aarch64`)
- macOS Intel (`x86_64`)
- macOS Universal
- Linux `x86_64`
- Linux `aarch64`

Linux release 同时包含 AppImage、deb 和 rpm updater targets。macOS Universal DMG 还会用于后续 Homebrew Cask 更新。

依赖缓存由 `.github/actions/host-build-cache` 统一配置。CI 在可信的 `main` push 上构建并保存缓存，后续 tag 可读取默认分支缓存；不同 tag 之间不能互读各自生成的缓存。CI 与 release 按相同平台标识使用 `shared-key`，Rust 工具链、依赖与环境仍参与键计算，workspace 指向仓库根目录的 `target`，不缓存宿主业务 crate。Windows/Linux 缓存实际的 Tauri 工具目录；macOS 的 create-dmg 脚本随 bundler 写入 `target/.../bundle/share`，不再缓存不存在的 `~/Library/Caches/tauri`。首次启用或工具链变化仍可能冷编译，实际提速需由后续远端构建确认。

Universal 不再从源码重编两套宿主。ARM 和 Intel release job 在本轮构建后通过 `scripts/release/macos-universal.cjs --mode stage` 上传宿主与 Go sidecar 的薄二进制，附带版本、commit、workflow run ID 和 SHA-256。Universal job 等待这两个 job 完成，使用同一 run 的具名 artifact；`--mode merge` 校验身份、文件哈希、实际架构，合并宿主和 sidecar，并逐片提取比对原始内容，再运行官方 `tauri bundle --target universal-apple-darwin`，沿用签名、资源和 DMG 流程。缺失或不匹配直接失败，不回退到其他版本或旧产物。CI matrix 中的 Universal 源码构建仍保留，用于独立兼容性验证。

ARM 和 Intel job 执行 `tauri build`，Universal job 执行 `tauri bundle`；三个 job 随后都会执行 `scripts/release/inject-dmg-readme.cjs`，用 `hdiutil` 重建 DMG（保留 `.app` 原样）并写入 `安装与常见问题.txt`，同时固定窗口中三个图标的位置：Cockpit Tools 与 Applications 保持同一行（`180,150` / `480,150`），保证“把图标拖到右侧安装”的动线不变；说明文件单独放在下一行（`330,330`），窗口尺寸随之调整为 660×490、图标 112px、文字 14pt。Tauri bundler 没有“向 DMG 追加文件”的配置项，因此这里重建的是 bundler 产物本身，而不是在打包阶段插文件；重建后 DMG 文件名与 Tauri 约定一致（`<productName>_<version>_<arch>.dmg`），供 `stage_release_assets.cjs` 正常规范化。说明文案模板位于 `scripts/release/assets/dmg-readme.txt`，修改后需与 `README.md` / `README.en.md` 的 Gatekeeper 排查章节保持一致。

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

随后重建包含安装说明的 DMG，再由 `macos-signing.cjs finalize` 签名并提交最终 DMG 公证，等待上限 30 分钟。校验覆盖原应用、解压后的自动更新应用和最终 DMG 内的应用，要求正确 Developer ID 团队、hardened runtime、有效公证票据及 Gatekeeper 认可；最终 DMG 本身也需通过签名、票据和 Gatekeeper 检查。全部通过后才进入资产暂存、上传与校验和生成。Universal 合并后仍由 Tauri 重新签名，不能复用合并前的签名。

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

`upload-checksums` job 会重新下载该版本的 release assets，对文件逐个计算 SHA-256，并上传：

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

## 7. 推荐发版顺序

1. 更新 `package.json` 版本。
2. 更新 `CHANGELOG.md` 与 `CHANGELOG.zh-CN.md`，确保存在对应版本段落。
3. 执行：

```bash
npm run sync-version
npm run release:preflight
```

4. 提交并合并发布所需改动。
5. 从期望发布的 commit 创建 `v<version>` 标签，先推送分支，再推送标签。
6. 检查 GitHub Actions 的 release workflow 完整成功。
7. 检查 GitHub Release 的平台 assets、target manifests、`latest.json` 和 `SHA256SUMS.txt`。
8. 检查 Homebrew Cask 自动 PR 的版本和 SHA-256 是否与 Universal DMG 一致。

按项目 `AGENTS.md`，发布动作以远端分支和对应版本标签均更新为完成条件。安装包构建、GitHub Release、updater manifests、checksums 与 Homebrew Cask 更新属于后置异步流程；报告时必须区分“分支与标签已发布”和“安装包已公开可下载”，不得把仍在构建的产物描述为已经可用。

## 8. 草稿发布与公开校验

当前 workflow 先创建 draft Release。全部平台构建、完整 manifests、checksums 和草稿资产校验完成后，`publish-release` 才公开 Release 并设置为 latest，随后检查公共下载地址；公开发布后不再补齐缺失平台来绕过校验。

GitHub 公共 latest 路由可能晚于 Release 状态变更。如果公开校验仍返回旧版本，应核对 Release API、公共下载重定向与本轮清单内容，保留版本、签名和资产归属校验；确认传播完成后可单独重跑发布校验 job。

同版本撤回重发必须经用户明确授权，先核对并清理旧同版本 Release 和标签，再从新提交重建标签。发布后应额外比较公开清单与新 Release 资产的内容哈希，避免同版本旧缓存仅凭版本号通过检查。已安装同版本的用户需要重新下载安装，不会因同版本号重发而自动升级。

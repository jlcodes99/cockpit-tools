# Release Process

本文档描述仓库当前由 `.github/workflows/release.yml` 执行的发布流程。发布行为以 workflow 和 `scripts/release/` 下的脚本为准；如果两者与本文不一致，应先修正文档或 workflow，再发版。

## 1. 发布前检查

在仓库根目录执行：

```bash
npm run release:preflight
```

当前 preflight 依次执行：

1. `node scripts/check_locales.cjs`
2. `npm run typecheck`
3. `npm run build`
4. `cargo check`（`src-tauri`）
5. `cargo test --lib`（`src-tauri`，`RUST_TEST_THREADS=1`）

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

Tauri release build 使用仓库配置的 updater signing secrets：

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

这里的 signing 指 Tauri updater artifact 签名；不要把它描述成已经完成 Apple notarization 或 Windows Authenticode，除非 workflow 另外明确实现了这些步骤。

## 4. Release assets 与 updater manifests

每个平台先通过 `scripts/release/stage_release_assets.cjs` 规范化允许上传的 release assets，再用 `scripts/release/build_target_latest_json.cjs` 生成各 target 的 updater manifest。

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
5. 从期望发布的 commit 创建 `v<version>` 标签并推送标签。
6. 检查 GitHub Actions 的 release workflow 完整成功。
7. 检查 GitHub Release 的平台 assets、target manifests、`latest.json` 和 `SHA256SUMS.txt`。
8. 检查 Homebrew Cask 自动 PR 的版本和 SHA-256 是否与 Universal DMG 一致。

仅有远端 branch 和 tag 并不代表发布已经成功。正式完成应以 release workflow 成功、预期 assets/manifests 可用以及 checksum 生成完成为准。

## 8. 当前已知的发布状态问题

当前 workflow 会在所有平台构建完成前就把 staged release 公开并标记为 latest，再在后续 job 中补齐完整 updater state 和 checksums。这样如果某个平台中途失败，公开 release 可能短时间或持续处于不完整状态。

该问题应独立修复，不应通过文档把它描述成推荐设计。修复目标是：release 在所有平台 assets、完整 manifests 和 checksums 验证完成前保持 draft，最后一次性发布并再做公开 URL 验证。

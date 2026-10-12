# Refreshing a remote Codex model catalog

## 中文

如果 Cockpit API 服务已经提供新模型，但通过该服务连接的远端 Codex CLI 在 `/model` 中看不到它，先检查客户端是否设置了 `model_catalog_json`。这个配置指向客户端磁盘上的静态目录；升级 Cockpit 不会自动更新另一台机器上的自定义文件。

这不是模型路由的证明：普通 `/v1/models` 只返回模型名称。Codex 需要 `/v1/models?client_version=<CLI版本>` 返回的完整 `{ "models": [...] }` 元数据，包含指令、推理选项、上下文限制与工具能力。不要把新模型映射到旧模型，也不要仅修改 `slug` 或手工补一个模型名称。

仓库提供一个无需额外依赖的手动补全工具：`scripts/refresh-codex-model-catalog.cjs`，需要 Node.js 20+。在持有静态目录的客户端机器上执行；可单独复制这个脚本，无需安装整个应用。

1. 运行 `codex --version`，检查实际生效的 `config.toml`（包括 profile 覆盖），找出 `model_catalog_json` 对应的文件。工具不会搜索或修改配置。
2. 将客户端的 Cockpit API Key 放入指定的环境变量。不要把密钥作为命令参数，也不要粘贴到 issue/PR 中。Linux/macOS 可使用不回显的输入：

   ```bash
   read -rsp 'Cockpit API key: ' CODEX_GATEWAY_API_KEY
   printf '\n'
   export CODEX_GATEWAY_API_KEY
   ```

3. 先预览，将示例中的目录、服务地址和版本替换为客户端的实际值：

   ```bash
   node scripts/refresh-codex-model-catalog.cjs \
     --catalog /absolute/path/to/models.json \
     --base-url https://gateway.example.com/v1 \
     --client-version 0.153.4 \
     --model gpt-6.1-sol \
     --api-key-env CODEX_GATEWAY_API_KEY \
     --dry-run
   ```

4. 确认输出的 `added` 后，去掉 `--dry-run` 执行。重复 `--model` 可以指定多个缺失模型。对于受信任内网中的明文 HTTP 服务，必须额外传 `--allow-insecure-http`；HTTPS 更安全，HTTP 会在网络上传输 bearer 密钥。工具禁止跟随重定向。
5. 查看输出中的 `backupPath` 并保留备份。运行 `codex debug models` 检查 CLI 实际加载的目录。API 目录中可见并不证明推理调用可用；还应单独验证目标模型的真实 Responses 请求。
6. 等已有任务完成，再退出并恢复原会话。静态目录在 Codex 启动时加载，旧窗口的 `/model` 不会因为磁盘文件改变而自动刷新。新窗口可用 `/model` 选择模型及其目录提供的推理档位。

   ```bash
   codex resume -m gpt-6.1-sol
   ```

   在选择器中选择原会话；并发会话较多时，不要用 `--last` 猜测目标。需要 Ultra 时，仅在真实 Codex 目录支持该档位的情况下增加 `-c 'model_reasoning_effort="ultra"'`。

工具只添加指定的缺失条目，完整复制网关提供的元数据，不更新或删除任何已有模型。原有 Astra Ultra、自定义条目与顶层字段都会保留；已有条目即使过期也不会被覆盖。`--dry-run` 不写文件；没有新增模型时不请求 API、不生成备份。缺失模型、无效目录、过新的最低客户端版本、鉴权失败与并发修改都会阻止替换。

实际写入前，工具创建原文件的独占备份，并通过同目录临时文件原子替换，保留 POSIX 权限位、所有者和所属组；不保证保留 Windows ACL 或扩展属性。工具拒绝目录和符号链接，使用 `.refresh.lock` 防止多个该工具同时写入，并在替换前检查文件是否被其他程序修改。这个检查不能消除最后一次检查与替换之间、不遵守锁的外部写入者产生的竞态，因此刷新时应避免其他程序同时编辑该文件。强制终止进程可能留下锁；先确认没有刷新进程运行，再处理遗留锁。发生错误时保留已经生成的备份。工具不会修改 `config.toml`、设置默认模型、发起付费推理请求、部署后台同步或重启任务。

## English

When Cockpit advertises a model but a remote Codex CLI omits it from `/model`, check the client's effective `model_catalog_json`, including profile overrides. A custom catalog is a local, startup-loaded file; upgrading the gateway does not distribute it to another machine.

Use `scripts/refresh-codex-model-catalog.cjs` on that client with Node.js 20+. Set the gateway credential in an environment variable, then run the dry-run example above with the actual catalog path, API Base URL, CLI version, and exact model ID. After reviewing `added`, remove `--dry-run`. Repeat `--model` for multiple missing IDs. Non-loopback plain HTTP requires the explicit `--allow-insecure-http` option; prefer HTTPS. Redirects are never followed.

The tool fetches the versioned Codex `{models: [...]}` response, not the ordinary OpenAI `{data: [...]}` list. It adds only selected missing entries, copying their full metadata without fabricating capabilities or aliasing new models to old ones. All existing model entries and top-level fields remain intact; it intentionally does not update existing entries. It rejects missing/hidden models, invalid metadata, incompatible minimum client versions, symlinks, and concurrent file changes. Authentication errors do not print the credential or response body.

Dry-run writes nothing. If all selected entries exist, the tool does not contact the gateway or create a backup. Changes use an exclusive backup and a same-directory atomic replacement, preserving POSIX mode, owner, and group; Windows ACLs and extended attributes are not guaranteed to be preserved. An advisory lock prevents concurrent runs of this tool; other writers are checked immediately before replacement. There remains a race between the final check and replacement for writers that do not honor the lock, so avoid editing the file concurrently. An interrupted run can leave a lock; confirm that no refresh is running before removing a stale lock. Already-created backups are retained on failure.

Keep the reported backup and check `codex debug models`. Catalog visibility alone does not establish inference access; verify an actual Responses call separately. Let active tasks finish before restarting/resuming the intended session. The tool does not alter configuration, change defaults, call inference, run background sync, or restart processes. Use `codex resume -m gpt-6.1-sol` to select the original session. Add `-c 'model_reasoning_effort="ultra"'` only when the gateway's actual Codex catalog supports Ultra. Avoid `--last` when concurrent sessions make the target ambiguous.

## References

- [Codex gateway catalog distribution and restart behavior](https://learn.chatgpt.com/docs/enterprise/roll-out-a-gateway)
- [Codex configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)
- [Codex model selection and Ultra mode](https://learn.chatgpt.com/docs/models)

Run the regression tests with:

```bash
npm run test:codex-model-catalog-refresh
```

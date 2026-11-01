# Codex Windows sandbox ownership recovery (temporary workaround)

中文说明见下半部分。 Related: [Cockpit #2972](https://github.com/jlcodes99/cockpit-tools/issues/2972), [Codex #57617](https://github.com/openai/codex/issues/57617).

## Scope and cause

This is an **opt-in workaround for a Codex upstream Windows sandbox issue**, not a change to Cockpit's launch behavior. Use it only when the current setup failure is `helper_sandbox_lock_failed`, with `open directory ... .sandbox-bin ... NtCreateFile: Access is denied (os error 5)`, and the Codex provisioning service runs as LocalSystem. It is not a general fix for all “Windows setup incomplete” errors.

Check the current error and service first, rather than relying on an old report:

```powershell
$codexHome = if ($env:CODEX_HOME) { $env:CODEX_HOME } else { Join-Path $env:USERPROFILE '.codex' }
Get-Item -LiteralPath "$codexHome\.sandbox\setup_error.json" | Select-Object LastWriteTime
Get-Content -LiteralPath "$codexHome\.sandbox\setup_error.json"
Get-CimInstance Win32_Service | Where-Object Name -Like 'CodexSandboxService*' | Select-Object Name, StartName
Get-Acl -LiteralPath "$codexHome\.sandbox-bin" | Format-List Owner, AccessToString
```

The reproduced directory had a protected DACL: SYSTEM and Administrators had Modify, the interactive user had Modify plus ChangePermissions, and CodexSandboxUsers had ReadAndExecute. The directory was owned by the interactive user.

Codex's [`lock_sandbox_dir`](https://github.com/openai/codex/blob/main/codex-rs/windows-sandbox-rs/src/setup_provisioning.rs) opens the directory with `READ_CONTROL | WRITE_DAC` in ProvisionOnly mode, but reconstructs SYSTEM's ACE without `WRITE_DAC`. Temporarily adding SYSTEM's ChangePermissions right allowed setup to complete; setup then removed it again. Making the service account (SYSTEM) the directory owner retains Windows' implicit owner right to change its DACL across these refreshes, without granting the sandbox users new rights.

## Inspect, preview, repair

Use Windows PowerShell 5.1 or PowerShell 7 on Windows, from a reviewed checkout of this repository. The script is **read-only by default** and does not elevate itself. Paths must be absolute local disk paths, without junctions or symlinks in any component. Pass the affected instance's actual CODEX_HOME, especially when using multiple instances.

```powershell
.\scripts\windows\codex-sandbox-owner.ps1 -CodexHome $codexHome
.\scripts\windows\codex-sandbox-owner.ps1 -CodexHome $codexHome -Repair -BackupPath "$env:USERPROFILE\codex-sandbox-owner-before.json" -WhatIf
```

For repair, open PowerShell **as administrator**, set `$codexHome` to the same affected user's absolute path (do not accidentally select a different administrator's profile), and run:

```powershell
.\scripts\windows\codex-sandbox-owner.ps1 -CodexHome $codexHome -Repair -BackupPath "$env:USERPROFILE\codex-sandbox-owner-before.json"
```

Keep the backup. An existing backup is never overwritten. The tool enables SeRestorePrivilege only during the operation and restores its previous state afterward. It opens an existing directory with a no-reparse native handle, refuses mapped drives/relative paths, and records the volume/file ID for rollback. It changes only that directory's owner to SYSTEM; its DACL, children, contents, Codex configuration and account data are not rewritten. It does not use recursive ACL grants or disable the sandbox. `SystemOwned` reports ownership only, not overall sandbox health.

Retry Windows setup, then run a harmless sandbox command more than once and check the current setup log. Do not delete setup markers to force a retry. Other error codes or continued failures need separate diagnosis.

## Updates and removal

The workaround does not patch Codex executables, its installer, update mechanism, version files or configuration format. It installs no task, service, watcher or launch hook and will not repeatedly override an upstream fix. It is intentionally isolated to two PowerShell files and this guide so Cockpit can remove it once Codex fixes the upstream issue.

Removing these files does not itself revert an already changed directory owner. If ownership should also be restored after the upstream fix, use the saved backup in an administrator PowerShell session:

```powershell
.\scripts\windows\codex-sandbox-owner.ps1 -CodexHome $codexHome -RestoreBackup "$env:USERPROFILE\codex-sandbox-owner-before.json" -WhatIf
.\scripts\windows\codex-sandbox-owner.ps1 -CodexHome $codexHome -RestoreBackup "$env:USERPROFILE\codex-sandbox-owner-before.json"
```

Rollback changes only ownership and preserves any newer DACL installed by Codex. It refuses a backup for a different path or directory identity; if an update recreates the directory, do not force an old backup onto it. No compatibility claim is made about untested future Codex versions.

## Validation

Run isolated native ownership/rollback tests in an administrator PowerShell session:

```powershell
.\scripts\windows\codex-sandbox-owner.test.ps1
```

Tests cover read-only inspection, WhatIf, ownership repair, unchanged DACL/child security and content, idempotence, owner rollback, backup collision, wrong directory identity, junction rejection and relative-path rejection. They create and clean up only temporary fixtures.

On Windows with OpenAI.Codex 26.1027.3384.0 (bundled runner 0.166.0-alpha.5.1), the workaround was applied to an affected directory. Two subsequent sandbox echo executions succeeded, both setup refreshes logged `errors=[]`, SYSTEM ownership persisted, and the existing DACL was unchanged. The affected user subsequently confirmed that the local client issue was fixed. Behavior across a future Codex update has not been verified.

---

## 中文说明

这是 **Codex 上游问题的手动临时恢复方案**，关联 [Cockpit #2972](https://github.com/jlcodes99/cockpit-tools/issues/2972) 和 [Codex #57617](https://github.com/openai/codex/issues/57617)。不能把“Windows 设置未完成”的所有情况都归为同一原因，也不能据此认定 Cockpit 启动逻辑有问题。

本机已复现：SYSTEM 只有 Modify，而 Codex 的初始化需要 WRITE_DAC。临时补上该权限可完成初始化，但 Codex 随后又会移除它。工具将**这一个 `.sandbox-bin` 目录的所有者**改为运行沙箱服务的 SYSTEM，使其在刷新后仍能管理目录 DACL；不递归修改权限，也不扩大沙箱账号权限。

先核对当前错误、时间戳及服务身份，再用上面的命令检查和预览。修复与回滚需要管理员 PowerShell；工具不自行提权，也不自动随 Cockpit 启动执行。多实例必须传入真正受影响实例的 CODEX_HOME。原所有者会先备份，备份不可覆盖，并绑定目录的实际身份，便于安全回滚。

工具不修改 Codex 安装文件、更新机制或配置格式，不注册后台任务，不会不断覆盖上游修复。Codex 修复后，Cockpit 可直接删除这两个独立脚本和本说明。**删除工具不会自动撤销已经发生的所有者调整**；如需撤销，可先用上面的 RestoreBackup 命令仅恢复所有者，同时保留 Codex 更新后的 DACL。若目录已被更新重建，工具会拒绝套用旧备份。

已通过临时目录的原生回归测试；在受影响本机连续两次沙箱执行及刷新均成功，SYSTEM 所有者保持不变。受影响用户随后确认本地客户端问题已修复。未来 Codex 更新尚未验证，不将该结果夸大为对所有版本的保证。

# Codex 本地项目清单恢复（本机补充工具）

此脚本补充 Cockpit 1.3.75 会话可见性修复未覆盖的项目清单断层。它是独立的 Python 标准库工具，**没有替换已安装的 Cockpit 可执行文件**。

## 场景

`state_5.sqlite.projects` 仍存在项目，而 `.codex-global-state.json.local-projects`、旧新项目 ID 映射、显式会话归属缺失；普通 provider/路径可见性修复后，侧栏仍无项目。

## 运行

只读预览（可在 Codex 运行时执行）：

```powershell
python scripts/repair_codex_local_projects.py
```

完全退出 Codex 后应用（运行中会拒绝写入）：

```powershell
python scripts/repair_codex_local_projects.py --apply
```

应用前备份 JSON 和 SQLite 在线备份到 `CODEX_HOME/backups/local-project-visibility-时间戳`；备份路径会打印。使用 `--codex-home` 显式指定非默认数据目录，默认 `~/.codex`。

完全退出 Codex 后回滚：

```powershell
python scripts/repair_codex_local_projects.py --rollback "备份目录"
```

回滚只在字段仍等于本次写入值时恢复原值，不用旧数据库覆盖后来生成的新会话。

## 保护边界

- 不读取/改动 auth.json、API Key、OAuth、浏览器登录信息、模型 provider 或账号身份。
- 不恢复其他账号的云项目；当前缓存里的云项目不改归属。
- 只恢复数据库中存在且本地目录有效的独立项目，尊重待删除项目和明确的 projectless 会话。
- 恢复旧 ID 映射，避免生成重复项目；新旧 SQLite 会话归属与本地目录投影同步。
- 不改会话正文、rollout 文件、归档状态、来源类型、创建者或更新时间；不把 guardian/子 agent 放到侧栏。
- 同一数据目录里的独立本地项目不增加账号绑定；不同 CODEX_HOME/独立多开实例不是同一份历史。
- 不承诺跨账号云项目同步，也不重写 encrypted_content 以绕过原上下文限制。

## 测试

```powershell
python -B -m unittest discover -s tests -p test_repair_codex_local_projects.py -v
```

13 项测试覆盖恢复、云项目隔离、旧 ID、已删除/缺目录、projectless、子 agent 排除、重复运行、并发保护、两套索引、保留新聊天的条件回滚和 Windows 路径。

# Cockpit Tools 编译与打包硬性规范（避坑备忘录）

> **【血的教训 · 严禁违背】**
> **绝对禁止在本项目中直接执行 `cargo build --release` 来试图构建发布可执行文件！**

---

## 核心故障现象与根因

### 1. 现象（"编译了一个 debug 的主页面没有的白屏程序"）
如果直接在根目录或 `src-tauri` 目录执行 `cargo build --release --bin cockpit-tools`：
- 生成的 `cockpit-tools.exe` 大小仅约 **88 MB**（正常包含前端的二进制约为 **93.6 MB**，缺失整整 5.3 MB 的前端静态资源）；
- 打开程序后，主窗口是一个**纯白空白窗口**（内存占用仅 13 MB，正常渲染的主界面内存占用在 80~200 MB 以上）；
- **根因**：Tauri 项目在 `tauri.conf.json` 中配置了 `"devUrl": "http://localhost:1420"`。直接使用 `cargo build` 时，Tauri 宏会保留开发期配置，将 Webview 指向开发服务器 `http://localhost:1420`；而此时本地根本没有启动 Vite 开发服务器，导致窗口载入失败全白！

### 2. 为什么开发者/AI 容易误入歧途？
- 在 Windows 环境下，`cargo.exe` 位于用户目录 `%USERPROFILE%\.cargo\bin`，有时未被包含在全局 Shell 的 `PATH` 中；
- 运行 `npm run tauri build` 时，Tauri CLI 尝试调用 `cargo metadata`，若 PATH 缺失会报错：`program not found`；
- 很多人一看到这个报错，偷懒直接用绝对路径去跑 `$env:USERPROFILE\.cargo\bin\cargo.exe build --release`，从而掉入**无主页面（白屏）**的致命陷阱！

---

## 唯一正确的发布编译命令

本项目已在 `scripts/tauri.cjs` 中配置了工具链自愈逻辑（自动寻找 `%USERPROFILE%\.cargo\bin`、`Go\bin` 以及 Visual Studio 的 `vcvars64.bat`）：

### 1. 编译并打包单可执行文件（推荐日常部署）：
```powershell
npm run tauri build -- --no-bundle
```
- 该命令会自动触发 `beforeBuildCommand`（即 `npm run build`，编译 TS 与 Vite 前端到 `dist/`）；
- 自动剔除 `devUrl`，强制将 `frontendDist`（`dist/` 中的 HTML/CSS/JS）内嵌打包至二进制中；
- 产物路径：`target/release/cockpit-tools.exe`（大小约 93.6 MB）。

### 2. 构建包含完整安装包（NSIS / MSI）：
```powershell
npm run tauri build
```

---

## 二进制有效性三步自检（动刀后必验）

在将新生成的 `cockpit-tools.exe` 复制到部署目录 `C:\Users\zouta\AppData\Local\Cockpit Tools\` 前，必须按如下标准自检：

1. **体积核对**：
   - 包含前端的正常体积：**≥ 93 MB**（如 93,607,936 字节）；
   - 若只有 **88 MB**，说明犯错走了直接 `cargo build`，严禁发布！
2. **进程拉起内存检查**：
   - 运行后执行 `Get-Process -Name cockpit-tools`；
   - 正常加载前端的主进程 WorkingSet 必须达到 **80 MB 以上**；若只有 13 MB，必定是白屏！
3. **子进程检查**：
   - 正常运行必须伴随 `msedgewebview2.exe` 的渲染进程启动。

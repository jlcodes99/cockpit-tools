# Windows 桌面软件制作方法指南（Tauri / Electron）

> 面向想从零开发一个 Windows 可安装桌面软件的开发者。本文以开源项目实践为目标，说明如何规划、开发、打包、发布一个 `.exe` / `.msi` 桌面应用。

## 目录

- [1. 你要做的是什么软件](#1-你要做的是什么软件)
- [2. 技术方案选择](#2-技术方案选择)
- [3. Windows 开发环境准备](#3-windows-开发环境准备)
- [4. 使用 Tauri 创建桌面应用](#4-使用-tauri-创建桌面应用)
- [5. 推荐项目结构](#5-推荐项目结构)
- [6. 前端界面开发](#6-前端界面开发)
- [7. 本地能力开发](#7-本地能力开发)
- [8. 配置、账号和数据存储](#8-配置账号和数据存储)
- [9. 打包成 Windows 安装包](#9-打包成-windows-安装包)
- [10. 发布到 GitHub Releases](#10-发布到-github-releases)
- [11. 开源仓库应该包含哪些文件](#11-开源仓库应该包含哪些文件)
- [12. 后续功能扩展方向](#12-后续功能扩展方向)

---

## 1. 你要做的是什么软件

在开始写代码之前，先明确软件定位。

例如：

- AI 账号管理工具
- 本地文件管理工具
- API Key 管理工具
- 软件启动器
- 多开实例管理工具
- 开发者工具箱
- 内部效率工具

建议先写一个最小可行版本（MVP）：

```text
1. 有一个可打开的 Windows 窗口
2. 有基础导航菜单
3. 可以保存设置
4. 可以读写本地文件
5. 可以打包成 .exe 或 .msi
```

不要一开始就做太复杂。先把软件跑起来，再逐步加功能。

---

## 2. 技术方案选择

### 方案 A：Tauri（推荐）

Tauri 使用 Web 前端构建界面，使用 Rust 提供本地能力。

适合：

- 轻量桌面软件
- 开发者工具
- 本地账号/配置管理
- 需要读写文件、调用命令行、启动外部程序的软件

优点：

- 安装包较小
- 性能好
- 系统能力强
- 适合多平台：Windows / macOS / Linux

常见技术组合：

```text
React + TypeScript + Vite + Tauri + Rust
```

### 方案 B：Electron

Electron 使用 Chromium + Node.js 构建桌面软件。

适合：

- 快速开发
- 依赖 Node.js 生态较多的软件
- 团队更熟悉 JavaScript / TypeScript 的项目

优点：

- 开发简单
- 文档多
- 生态成熟

缺点：

- 安装包较大
- 内存占用通常更高

### 推荐结论

如果你想做类似 Cockpit Tools 这种桌面工具，推荐：

```text
Tauri + React + TypeScript
```

---

## 3. Windows 开发环境准备

### 必需工具

Windows 上建议安装：

```text
Git
Node.js LTS
Rust / rustup
Visual Studio Build Tools
VS Code / Cursor / WebStorm
```

### 安装 Node.js

下载地址：

```text
https://nodejs.org/
```

检查：

```powershell
node -v
npm -v
```

### 安装 Rust

下载地址：

```text
https://www.rust-lang.org/tools/install
```

检查：

```powershell
rustc --version
cargo --version
```

### 安装 Visual Studio Build Tools

Tauri 在 Windows 上构建时需要 MSVC 链接器。

可以通过 winget 安装：

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-package-agreements --accept-source-agreements --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

安装后确认存在：

```powershell
C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat
```

---

## 4. 使用 Tauri 创建桌面应用

### 创建项目

```powershell
npm create tauri-app@latest my-desktop-app
cd my-desktop-app
npm install
```

创建时可以选择：

```text
Framework: React
Language: TypeScript
Package manager: npm
```

### 开发模式运行

```powershell
npm run tauri dev
```

如果成功，会打开一个桌面窗口。

### 构建生产版本

```powershell
npm run tauri build
```

构建完成后，产物一般在：

```text
src-tauri/target/release/bundle/
```

或者：

```text
target/release/bundle/
```

常见 Windows 产物：

```text
.msi
.exe
```

---

## 5. 推荐项目结构

一个较清晰的桌面软件项目结构如下：

```text
my-desktop-app/
├─ src/                         # 前端代码
│  ├─ components/                # 通用组件
│  ├─ pages/                     # 页面
│  ├─ stores/                    # 状态管理
│  ├─ services/                  # 前端服务封装
│  ├─ utils/                     # 工具函数
│  └─ main.tsx
│
├─ src-tauri/                    # Tauri / Rust 后端
│  ├─ src/
│  │  ├─ main.rs
│  │  ├─ commands/               # Tauri commands
│  │  ├─ modules/                # 本地业务模块
│  │  └─ utils/                  # Rust 工具函数
│  ├─ tauri.conf.json
│  └─ Cargo.toml
│
├─ docs/                         # 项目文档
├─ scripts/                      # 构建/发布脚本
├─ public/                       # 静态资源
├─ package.json
├─ README.md
├─ LICENSE
└─ CHANGELOG.md
```

---

## 6. 前端界面开发

常用前端技术：

```text
React
TypeScript
Vite
Tailwind CSS
DaisyUI / shadcn/ui / Ant Design
Zustand / Redux
```

### 推荐页面

一个桌面工具通常包含：

```text
Dashboard 首页
Accounts 账号管理
Settings 设置
Logs 日志
About 关于
```

### 示例页面规划

```text
首页：显示当前状态、快捷操作
账号页：添加、删除、切换、刷新账号
设置页：配置数据目录、语言、端口、自动启动
日志页：查看运行日志和错误信息
关于页：版本号、开源协议、GitHub 地址
```

---

## 7. 本地能力开发

桌面软件和普通网页最大的区别是：桌面软件可以操作本地电脑。

常见能力包括：

- 读写本地文件
- 创建配置目录
- 启动外部程序
- 调用命令行
- 读取系统环境变量
- 扫描软件安装路径
- 托盘运行
- 开机自启
- 本地端口服务
- 自动更新

### Tauri Command 示例

Rust 后端：

```rust
#[tauri::command]
fn greet(name: String) -> String {
    format!("Hello, {}!", name)
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

前端调用：

```ts
import { invoke } from '@tauri-apps/api/core'

const message = await invoke<string>('greet', { name: 'Windows' })
console.log(message)
```

---

## 8. 配置、账号和数据存储

桌面工具通常需要保存用户配置。

### 推荐保存位置

Windows 上推荐使用应用数据目录：

```text
C:\Users\<用户名>\AppData\Roaming\YourApp
C:\Users\<用户名>\AppData\Local\YourApp
```

### 推荐数据文件

```text
settings.json      # 应用设置
accounts.json      # 账号列表
logs/              # 日志目录
backups/           # 备份目录
```

### 配置设计示例

```json
{
  "language": "zh-CN",
  "theme": "system",
  "autoStart": false,
  "dataDir": "default",
  "checkUpdateOnStart": true
}
```

### 账号数据示例

```json
[
  {
    "id": "account_001",
    "name": "Main Account",
    "platform": "codex",
    "createdAt": "2026-01-01T00:00:00Z",
    "tags": ["main", "work"]
  }
]
```

敏感信息建议加密保存，不要明文提交到 Git 仓库。

---

## 9. 打包成 Windows 安装包

### 基础打包命令

```powershell
npm run tauri build
```

### 常见输出

```text
target/release/bundle/msi/YourApp_x.x.x_x64_en-US.msi
target/release/bundle/nsis/YourApp_x.x.x_x64-setup.exe
```

### 常见问题

#### 1. rustc 版本太低

错误示例：

```text
requires rustc 1.88.0
```

解决：

```powershell
rustup update stable
rustc --version
```

#### 2. 找不到 link.exe

错误示例：

```text
linker `link.exe` not found
```

解决：安装 Visual Studio Build Tools，并启用 C++ 工作负载。

#### 3. 找不到 go

如果项目包含 Go sidecar，可能会出现：

```text
failed to start go build
program not found
```

解决：

```powershell
winget install --id GoLang.Go -e
```

#### 4. 没有签名私钥

错误示例：

```text
A public key has been found, but no private key
```

这通常影响自动更新签名。普通本地安装包可能已经生成，但正式发布建议配置签名。

---

## 10. 发布到 GitHub Releases

### 版本号管理

建议遵循语义化版本：

```text
1.0.0
1.0.1
1.1.0
2.0.0
```

### 发布流程

```text
1. 更新 package.json 版本号
2. 更新 CHANGELOG.md
3. 本地构建
4. 测试安装包
5. 创建 Git tag
6. 推送到 GitHub
7. 创建 GitHub Release
8. 上传 .msi / .exe
```

### Git 命令示例

```bash
git add .
git commit -m "docs: add Windows desktop app development guide"
git tag v1.0.0
git push origin main
git push origin v1.0.0
```

---

## 11. 开源仓库应该包含哪些文件

一个适合开源的桌面软件仓库建议包含：

```text
README.md              # 项目介绍和使用方法
LICENSE                # 开源协议
CONTRIBUTING.md        # 贡献指南
CHANGELOG.md           # 更新日志
SECURITY.md            # 安全说明
CODE_OF_CONDUCT.md     # 社区行为准则，可选
docs/                  # 更多文档
.github/workflows/     # CI/CD 自动构建
```

### README.md 建议结构

```text
项目名称
项目截图
功能介绍
下载安装
源码运行
打包构建
常见问题
贡献指南
开源协议
```

### 开源协议选择

常见协议：

| 协议 | 适合场景 |
|---|---|
| MIT | 最宽松，适合大多数工具项目 |
| Apache-2.0 | 更完整，包含专利授权 |
| GPL-3.0 | 要求衍生项目也开源 |
| AGPL-3.0 | 对网络服务也要求开源 |

如果你希望别人自由使用、修改、二次开发，推荐：

```text
MIT 或 Apache-2.0
```

---

## 12. 后续功能扩展方向

当基础桌面软件完成后，可以继续加入：

```text
自动更新
托盘图标
开机自启
多语言 i18n
暗色模式
本地日志系统
配置备份/恢复
WebDAV / 云同步
插件系统
本地 HTTP API
WebSocket 联动
账号加密存储
GitHub Actions 自动打包
```

---

## 最小实践路线

如果你是新手，建议按这个顺序做：

```text
第 1 天：创建 Tauri 项目，跑出窗口
第 2 天：做首页和设置页
第 3 天：实现配置文件保存
第 4 天：实现本地文件读写
第 5 天：打包成 .msi / .exe
第 6 天：写 README 和 LICENSE
第 7 天：发布到 GitHub Releases
```

---

## 一句话总结

做 Windows 桌面软件的核心流程是：

```text
明确功能 → 选择技术栈 → 创建项目 → 开发界面 → 实现本地能力 → 打包安装包 → 写文档 → 开源发布
```

推荐技术栈：

```text
Tauri + React + TypeScript + Rust
```

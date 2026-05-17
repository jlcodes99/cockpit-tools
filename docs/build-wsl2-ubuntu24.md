# Ubuntu 24.04 编译指导

本文档用于在 Ubuntu 24.04 环境中本地编译 Cockpit Tools。WSL2 的 Ubuntu 24.04 也按本文档执行。

注意：项目官方发布目标是 macOS 和 Windows，Linux/Ubuntu 不是官方发布目标。本文档用于本地验证和自用构建。

## 系统要求

操作系统：

```bash
cat /etc/os-release
uname -a
```

确认输出中包含 Ubuntu 24.04：

```text
VERSION_ID="24.04"
```

如果在 WSL2 中编译，可额外确认：

```bash
grep -i microsoft /proc/version
```

仓库目录约定：

```text
~/codes/cockpit-tools
```

Node.js 要求：

```text
nvm 0.40.4
Node.js 24
npm 9+
```

Rust 要求：

```text
Rust stable
Cargo 可用
```

Tauri Linux 编译要求：

```text
GTK 3
WebKitGTK 4.1
AppIndicator
OpenSSL
pkg-config
基础 C/C++ 编译工具链
```

## 安装依赖

可选：如果 Ubuntu 官方源下载速度慢，可以使用阿里云 Ubuntu 镜像加速系统依赖下载：

```bash
sudo cp /etc/apt/sources.list.d/ubuntu.sources \
  /etc/apt/sources.list.d/ubuntu.sources.ubuntu24-build-bak

sudo sed -i \
  's#http://archive.ubuntu.com/ubuntu/#https://mirrors.aliyun.com/ubuntu/#g; s#http://security.ubuntu.com/ubuntu/#https://mirrors.aliyun.com/ubuntu/#g; s#https://mirrors.tuna.tsinghua.edu.cn/ubuntu/#https://mirrors.aliyun.com/ubuntu/#g' \
  /etc/apt/sources.list.d/ubuntu.sources

sudo apt-get update
```

安装 Ubuntu/Tauri 系统依赖：

```bash
sudo apt-get update

sudo apt-get install -y \
  build-essential \
  ca-certificates \
  curl \
  file \
  git \
  pkg-config \
  wget \
  libssl-dev \
  libgtk-3-dev \
  libwebkit2gtk-4.1-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libxdo-dev
```

检查 GTK 和 WebKitGTK：

```bash
pkg-config --modversion gtk+-3.0
pkg-config --modversion webkit2gtk-4.1
```

把仓库 clone 到 `~/codes`：

```bash
mkdir -p ~/codes
cd ~/codes

if [ ! -d cockpit-tools/.git ]; then
  git clone https://github.com/jlcodes99/cockpit-tools.git cockpit-tools
fi

cd ~/codes/cockpit-tools
test -f package.json && test -f src-tauri/tauri.conf.json && pwd
```

安装 Rust stable。如果已经安装过 Rust stable，并且 `rustc --version`、`cargo --version` 都能正常输出，可以跳过安装命令，只保留版本检查：

```bash
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable
fi

. "$HOME/.cargo/env"

rustc --version
cargo --version
```

如果 Rust 已安装但版本较旧，更新 stable 工具链：

```bash
rustup update stable
rustup default stable
rustc --version
cargo --version
```

安装或更新 nvm 到 `0.40.4`，并安装 Node.js 24。如果已经安装过 nvm `0.40.4` 和 Node.js 24，可以跳过安装命令，只保留版本检查：

```bash
export NVM_DIR="$HOME/.nvm"
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.4/install.sh | bash

. "$NVM_DIR/nvm.sh"
nvm --version
nvm install 24
nvm use 24
nvm alias default 24

node -v
npm -v
```

如果 nvm 已安装但版本不是 `0.40.4`，重新执行上面的 nvm 安装命令即可更新。

确认 nvm 是 `0.40.4`：

```bash
test "$(nvm --version)" = "0.40.4" && echo "nvm 0.40.4 OK"
```

确认 Node.js 是 24：

```bash
node -e '
const major = Number(process.versions.node.split(".")[0]);
if (major !== 24) {
  console.error(`Node.js ${process.versions.node} is not 24.x.`);
  process.exit(1);
}
console.log(`Node.js ${process.versions.node} OK`);
'
```

依赖安装完成后，关闭当前 shell 窗口，重新打开一个 Ubuntu 24.04 shell 窗口。这样 nvm 和 Cargo 的环境变量会从 shell 启动脚本自动加载，后面的命令可以更短。

新 shell 窗口打开后执行：

```bash
cd ~/codes/cockpit-tools

node -v
npm -v
nvm --version
cargo --version
```

确认 `node -v` 输出为 `v24.x.x`，`nvm --version` 输出为 `0.40.4` 后，再继续下面的编译步骤。

## 进行编译

以下命令默认都在新打开的 shell 中执行，并且当前目录是：

```bash
cd ~/codes/cockpit-tools
```

安装前端依赖：

```bash
npm ci --cache .npm-cache
```

不要用 `sudo npm ...` 编译项目。`sudo` 可能会切到系统自带的旧 Node.js，导致 Vite 报错。

编译前检查 Rust / Node.js 版本是否匹配本文档要求：

```bash
node - <<'NODE'
const major = Number(process.versions.node.split(".")[0]);
if (major !== 24) {
  console.error(`Node.js ${process.versions.node} is not 24.x.`);
  process.exit(1);
}
console.log(`Node.js ${process.versions.node} OK`);
NODE

cargo --version
```

本地 Ubuntu 24.04 编译时，先临时关闭 Tauri updater artifacts。

原因：`src-tauri/tauri.conf.json` 中配置了：

```json
"createUpdaterArtifacts": true
```

本地通常没有发布签名私钥。保持该配置会在打包完成后失败：

```text
A public key has been found, but no private key.
Make sure to set TAURI_SIGNING_PRIVATE_KEY environment variable.
```

编译前先备份配置并临时关闭：

```bash
cp src-tauri/tauri.conf.json src-tauri/tauri.conf.json.ubuntu24-build-bak

node - <<'NODE'
const fs = require("fs");
const file = "src-tauri/tauri.conf.json";
const config = JSON.parse(fs.readFileSync(file, "utf8"));

if (!config.bundle) {
  config.bundle = {};
}

config.bundle.createUpdaterArtifacts = false;
fs.writeFileSync(file, `${JSON.stringify(config, null, 2)}\n`);
NODE
```

确认已经关闭：

```bash
node - <<'NODE'
const fs = require("fs");
const config = JSON.parse(fs.readFileSync("src-tauri/tauri.conf.json", "utf8"));
console.log(config.bundle.createUpdaterArtifacts);
process.exit(config.bundle.createUpdaterArtifacts === false ? 0 : 1);
NODE
```

输出应为：

```text
false
```

执行完整编译和打包：

```bash
npm run tauri build
```

首次编译会下载 Rust crates，release 构建可能需要十几分钟。后续增量编译会快很多。

如果编译失败，先更新 Rust stable 和 Node.js 24，再重新安装 npm 包并重试：

```bash
rustup update stable
rustup default stable

nvm install 24
nvm use 24
nvm alias default 24

npm ci --cache .npm-cache
npm run tauri build
```

编译成功后检查二进制：

```bash
ls -lh target/release/cockpit-tools
```

检查安装包：

```bash
find target/release/bundle -maxdepth 2 -type f -printf '%p %s bytes\n'
```

常见产物：

```text
target/release/bundle/deb/Cockpit Tools_0.22.20_amd64.deb
target/release/bundle/rpm/Cockpit Tools-0.22.20-1.x86_64.rpm
target/release/bundle/appimage/Cockpit Tools_0.22.20_amd64.AppImage
```

如果只想验证二进制能否编译，不需要生成 deb/rpm/AppImage：

```bash
npm run tauri -- build --no-bundle
```

编译完成后恢复 `tauri.conf.json`：

```bash
mv src-tauri/tauri.conf.json.ubuntu24-build-bak src-tauri/tauri.conf.json
```

确认配置没有留下临时改动：

```bash
git diff -- src-tauri/tauri.conf.json
```

如果没有输出，说明配置已恢复。

## 开发模式

以下命令默认都在新打开的 shell 中执行，并且当前目录是：

```bash
cd ~/codes/cockpit-tools
```

开发模式会启动 Vite dev server 和 Tauri 应用：

```bash
npm run tauri dev
```

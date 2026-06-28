ARG DEBIAN_VERSION=bookworm

# ============================================================
# 1. base：通用构建基础层
# ============================================================
FROM debian:${DEBIAN_VERSION}-slim AS base

ENV DEBIAN_FRONTEND=noninteractive
ENV PATH="/root/.cargo/bin:/usr/local/go/bin:${PATH}"

# 让 cargo 和 tauri build 使用同一个 target 目录，便于复用缓存。
ENV CARGO_TARGET_DIR=/src/src-tauri/target

# 网络不稳定时提高成功率。
ENV CARGO_NET_RETRY=20
ENV CARGO_HTTP_TIMEOUT=300
ENV CARGO_HTTP_MULTIPLEXING=false

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    wget \
    git \
    file \
    bash \
    build-essential \
    pkg-config \
    libssl-dev \
    libgtk-3-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    patchelf \
    libsoup-3.0-dev \
    libwebkit2gtk-4.1-dev \
    javascriptcoregtk-4.1 \
    libjavascriptcoregtk-4.1-dev \
    libnm-dev \
    xdg-utils \
    && rm -rf /var/lib/apt/lists/*


# node js

RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
 && apt-get install -y nodejs

# Rust
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal

# Go：sidecars/cockpit-cliproxy 需要。
ARG GO_VERSION=1.26.0
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
      amd64) goarch="amd64" ;; \
      arm64) goarch="arm64" ;; \
      *) echo "unsupported arch: $arch" >&2; exit 1 ;; \
    esac; \
    wget -q "https://go.dev/dl/go${GO_VERSION}.linux-${goarch}.tar.gz" -O /tmp/go.tar.gz; \
    tar -C /usr/local -xzf /tmp/go.tar.gz; \
    rm /tmp/go.tar.gz; \
    go version

WORKDIR /src


# ============================================================
# 2. npm-deps：只缓存 npm 依赖
# package.json / package-lock.json 不变，这一层就不会重跑
# ============================================================
FROM base AS npm-deps

WORKDIR /src

COPY package.json package-lock.json* ./

RUN npm ci


# ============================================================
# 3. rust-deps：提前下载并编译 Rust/Go/Tauri 依赖
# 只要 Rust 相关文件不变，这一层就能缓存
# ============================================================
FROM npm-deps AS rust-deps

WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src-tauri ./src-tauri
COPY sidecars ./sidecars
COPY scripts ./scripts

# Rust 代码 include_str! 编译期会读取这些文件。
COPY CHANGELOG.md CHANGELOG.zh-CN.md ./
COPY src/locales ./src/locales

# Tauri 打包 / 图标资源可能需要。
COPY src/assets ./src/assets

# 先单独拉依赖；失败会更早暴露，不会等完整 tauri build。
RUN set -eux; \
    cargo fetch --locked

# 预编译 Rust 依赖和 Go sidecar。
# 这一步很慢，但缓存命中后，前端改动不会导致它重跑。
RUN set -eux; \
    cargo build --release --locked --manifest-path src-tauri/Cargo.toml


# ============================================================
# 4. builder：复制完整前端代码，最终 Tauri 打包
# ============================================================
FROM rust-deps AS builder

WORKDIR /src

# 复制前端构建需要的根目录配置文件。
COPY index.html ./
COPY vite.config.ts ./
COPY tsconfig.json ./
COPY tsconfig.node.json ./
COPY postcss.config.* ./
COPY tailwind.config.* ./
COPY components.json* ./

# 复制完整前端源码和 public。
COPY public ./public
COPY src ./src

# 再复制一次可能变化的发布/配置文件。
COPY announcements.json* ./
COPY remote-config.json* ./

# 确保 package 版本同步，然后最终打包。
# 因为前面 cargo build 已经产生 target 缓存，这里通常会快很多。
RUN set -eux; \
    npm run sync-version; \
    npx tauri build --ci --config src-tauri/tauri.ci.conf.json --bundles deb; \
    mkdir -p /out; \
    cp -a src-tauri/target/release/bundle /out/bundle; \
    find /out -maxdepth 4 -type f | sort


# ============================================================
# 5. runtime：运行依赖 + 构建好的 .deb
# ============================================================
FROM debian:${DEBIAN_VERSION}-slim AS runtime

ENV DEBIAN_FRONTEND=noninteractive
ENV WEBKIT_DISABLE_DMABUF_RENDERER=1
ENV NO_AT_BRIDGE=1
ENV DISPLAY=:1
ENV HOME=/home/app
ENV XDG_RUNTIME_DIR=/tmp/runtime-app

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    dbus-x11 \
    xdg-utils \
    libgtk-3-0 \
    libayatana-appindicator3-1 \
    librsvg2-2 \
    libsoup-3.0-0 \
    libwebkit2gtk-4.1-0 \
    javascriptcoregtk-4.1 \
    libjavascriptcoregtk-4.1-0 \
    libnm0 \
    fonts-dejavu \
    fonts-noto-cjk \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/src-tauri/target/release/bundle/deb/*.deb /tmp/cockpit-tools.deb

RUN apt-get update \
    && apt-get install -y --no-install-recommends /tmp/cockpit-tools.deb \
    && rm -f /tmp/cockpit-tools.deb \
    && rm -rf /var/lib/apt/lists/*

ARG APP_UID=1000
ARG APP_GID=1000

RUN set -eux; \
    groupadd -g "${APP_GID}" app; \
    useradd -m -u "${APP_UID}" -g "${APP_GID}" app; \
    mkdir -p /tmp/runtime-app /home/app; \
    chown -R app:app /tmp/runtime-app /home/app

COPY docker/run-cockpit-tools.sh /usr/local/bin/run-cockpit-tools
RUN chmod +x /usr/local/bin/run-cockpit-tools

USER app

CMD ["/bin/sh", "/usr/local/bin/run-cockpit-tools"]


# ============================================================
# 6. novnc：浏览器访问版
# ============================================================
FROM runtime AS novnc

USER root

RUN apt-get update && apt-get install -y --no-install-recommends \
    xvfb \
    x11vnc \
    fluxbox \
    novnc \
    websockify \
    procps \
    && rm -rf /var/lib/apt/lists/*

COPY docker/start-novnc.sh /usr/local/bin/start-novnc

RUN chmod +x /usr/local/bin/start-novnc \
    && chown app:app /usr/local/bin/start-novnc

USER app

EXPOSE 6080

CMD ["/bin/sh", "/usr/local/bin/start-novnc"]


# ============================================================
# 7. bundles：只导出安装包
# ============================================================
FROM scratch AS bundles
COPY --from=builder /out/ /
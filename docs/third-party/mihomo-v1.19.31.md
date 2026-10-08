# Mihomo v1.19.31 — third-party review record

Status: **not approved for bundling**. The host downloads an official pinned archive only when the user requests installation, or accepts that exact archive from disk. Normal builds do not bundle the engine.

## Upstream and pinned artifacts

- Release: [v1.19.31](https://github.com/MetaCubeX/mihomo/releases/tag/v1.19.31)
- Corresponding source: [tagged source archive](https://github.com/MetaCubeX/mihomo/archive/refs/tags/v1.19.31.tar.gz)
- License: [upstream GPLv3 LICENSE](https://github.com/MetaCubeX/mihomo/blob/v1.19.31/LICENSE)
- SHA-256 and sizes: [official GitHub release API](https://api.github.com/repos/MetaCubeX/mihomo/releases/tags/v1.19.31)
- Pinned manifest: [`sidecars/mihomo/upstream-assets.json`](../../sidecars/mihomo/upstream-assets.json)

Six target archives are pinned by their official GitHub asset digests. macOS and Linux use a gzip-compressed executable; Windows uses a ZIP containing one executable, normalized to `mihomo.exe`. No Cronet DLL is required. The x86_64 assets use the upstream amd64-v1 baseline. Upstream provides separate macOS architectures; universal bundling requires a separate build and verification decision.

## Installation and execution

Installation verifies the archive SHA-256 before extraction, bounds compressed and decompressed sizes, supports cancellation and timeouts, checks the engine version, and atomically publishes an immutable release. Active processes hold a release lease. A failed installation preserves the previous active pointer. Old sing-box records fail the new version/digest requirements and are never selected as Mihomo.

Downloads occur only after a user installation request, directly from GitHub with restricted HTTPS redirects. Page reads and app startup do not download the engine. The configuration and account proxy lifecycle are managed by the host; the engine runs as a separate background process. Windows launches hide the console window. No system proxy or TUN is enabled by installing the engine.

## Optional offline packaging

Verify an archive without executing or downloading it:

```sh
node scripts/verify-proxy-engine-archive.cjs <rust-target> <archive-path>
```

Prepare the verified executable:

```sh
COCKPIT_PROXY_ENGINE_TARGET=<rust-target> \
COCKPIT_PROXY_ENGINE_ARCHIVE=<verified-archive> \
npm run proxy-engine:prepare
```

`prepared.json` records the target, archive digest and executable digest. Bundling remains blocked by `distributionApproved: false`. Before changing this flag, review the corresponding source, license and dependency notices, build inputs, modifications and durable source availability for the intended distribution. Process separation alone is not a licensing conclusion. After that review, the existing explicit `proxy-engine:bundle-config` command can generate a per-target Tauri resources overlay. Default development and release builds do not include Mihomo.

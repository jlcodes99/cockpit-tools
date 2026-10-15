# sing-box v1.14.1 — third-party review record

> Historical record. The active proxy engine is now [Mihomo v1.19.31](mihomo-v1.19.31.md); the commands and runtime details below describe the former sing-box integration.

Status: **not approved for distribution**. This file records engineering facts and is not legal advice.

## Upstream

- Release: [v1.14.1](https://github.com/SagerNet/sing-box/releases/tag/v1.14.1)
- Source archive: [sing-box v1.14.1 source](https://github.com/SagerNet/sing-box/archive/refs/tags/v1.14.1.tar.gz)
- License: [upstream LICENSE](https://github.com/SagerNet/sing-box/blob/v1.14.1/LICENSE)
- Build workflow: [`.github/workflows/build.yml`](https://github.com/SagerNet/sing-box/blob/v1.14.1/.github/workflows/build.yml)
- Default build tags: [`release/DEFAULT_BUILD_TAGS`](https://github.com/SagerNet/sing-box/blob/v1.14.1/release/DEFAULT_BUILD_TAGS)
- Archive digests: [GitHub expanded assets](https://github.com/SagerNet/sing-box/releases/expanded_assets/v1.14.1)

The upstream license is GPLv3-or-later and includes an additional name/association restriction. The host project license is different. Process separation is an engineering boundary, not a legal conclusion. Before shipping, preserve the corresponding source, license text, dependency notices, build inputs, modification record, and a durable source-access path as required by the chosen distribution model.

## Pinned artifacts

The machine-readable list is [`sidecars/sing-box/upstream-assets.json`](../../sidecars/sing-box/upstream-assets.json). It pins six upstream archives and their SHA-256 values. Verify an archive without downloading or executing it:

```sh
node scripts/verify-proxy-engine-archive.cjs <rust-target> <archive-path>
```

Prepare a verified archive explicitly:

```sh
COCKPIT_PROXY_ENGINE_TARGET=<rust-target> \
COCKPIT_PROXY_ENGINE_ARCHIVE=<verified-archive> \
npm run proxy-engine:prepare
```

The preparation command does not download an archive. It extracts only an archive whose digest matches the manifest and refuses missing environment variables. It is not a distribution approval step.

The prepared directory also contains `prepared.json`, recording the target, pinned archive digest, and hashes of the files to bundle. After the corresponding-source and license review is complete, set `distributionApproved` to `true` in the pinned manifest through a reviewed project change. Then generate a Tauri configuration overlay for **one target**:

```sh
npm run proxy-engine:bundle-config -- <rust-target>
npm run tauri build -- --target <rust-target> --config .tmp/proxy-engine/tauri.<rust-target>.conf.json
```

The configuration maps the executable to `proxy-engine/` inside Tauri resources and includes `libcronet.dll` on Windows. The build command is an explicit packaging path; normal development and release builds do not include the engine. The generator rejects a missing or mismatched prepared target, changed binary, and the current unapproved manifest. It does not download or execute the engine. The macOS universal target is deliberately unsupported: upstream supplies separate architecture binaries, and a merged binary would need its own verification and signing review. The release workflows must be updated and their installer contents checked before claiming that official downloads include the engine.

## Platform-specific review notes

- Windows ZIP archives include `sing-box.exe` and `libcronet.dll`; shipping only the executable is not equivalent to shipping the upstream package.
- The upstream release has separate macOS x64 and arm64 archives, not a macOS universal archive. Any universal build must be produced and signed separately, then verified.
- The upstream build uses `with_naive_outbound`; the pinned build also needs review of `with_quic`, `with_utls`, and whether gRPC is present before presenting any URI combination as supported.
- The pinned Cronet source version in the upstream build metadata is `0d28acc44093df24b2526dea3d6ffefd6b0a54f0`.

## User-triggered installation

The proxy page can download the pinned official archive directly from GitHub or import the same archive from disk. This is distinct from bundling: `distributionApproved: false` continues to disable inclusion in our installer. No app startup or page read triggers downloads. The installer checks the pinned archive SHA-256, preserves available LICENSE/NOTICE files, requires Windows companion DLLs, and switches an immutable user-data release only after extraction and version validation. Active processes retain a lease on their release. One previous release is retained, older unused releases and abandoned staging directories are cleaned during installation. Source and licensing references remain linked above.

## Runtime boundary

The application passes configuration over stdin to `sing-box run -c stdin`; secrets are not command-line arguments or logs. The local mixed inbound is loopback-only, and Windows background launches use `CREATE_NO_WINDOW`. The desktop bridge intentionally has no local-user authentication because Chromium cannot reliably consume authenticated SOCKS launch flags; it is not a device or OS-user isolation feature.

# Tao Windows input deadlock backport

Base: crates.io `tao 0.34.8`, checksum `9103edf55f2da3c82aea4c7fab7c4241032bfeea0e71fa557d98e00e7ce7cc20`.

Backport: https://github.com/tauri-apps/tao/pull/1215 (merged as `c704261c519c58cfdd0bc2d58ba24e06a0b71c92`).
Only the upstream changes to `src/platform_impl/windows/{event_loop,keyboard,keyboard_layout,minimal_ime}.rs` are applied. Other runtime source and dependency versions match the published crate.

The patch moves `PeekMessageW` calls outside input locks to prevent synchronous window-procedure reentry from deadlocking keyboard and IME handling. Non-Windows behavior is unchanged.

Keep the upstream Apache-2.0 license. Remove this vendored override once the supported Tauri dependency range includes a released Tao version containing the fix. Do not edit the Cargo registry cache.

# Muda Windows menu command ID backport

Base: crates.io `muda 0.17.2`, checksum `7c9fec5a4e89860383d778d10563a605838f8f0b2f9303868937e5ff32e86177`.

Backport: [tauri-apps/muda#430](https://github.com/tauri-apps/muda/pull/430), merged as `1802a0d7d3c2e2f951d8d8212fb2e6cd4fa54e69` and released in `0.21.2`. Cockpit tracks the bug in [#3139](https://github.com/jlcodes99/cockpit-tools/issues/3139).

The Windows implementation and its menu-event tests are confined to `src/platform_impl/windows/`. Windows menu commands and accelerator entries use 16-bit IDs that cycle from 1000 through 61439. Root menu identities and generated public IDs retain the original counter. The backport uses the existing submenu handles for native menu operations.

Run the regression on Windows with `cargo test --manifest-path tests/tray_command_capacity/Cargo.toml --test windows -- --nocapture`. It creates a hidden window, attaches the same native popup subclass as the tray, and checks the Quit event after 70,000, 140,000, and 210,000 cumulative item creations.

Verified on Windows 11 ARM64 with a native ARM64 test binary and an emulated x64 test binary. Both variants of the original crate delivered Quit at startup and missed it at all three checkpoints. Both variants of the patched crate delivered Quit at every checkpoint. The Cockpit Windows x64 compile check also passed with the override.

Keep the upstream Apache-2.0 and MIT licenses. Remove the override when the supported Tauri and tray-icon versions use a released Muda that includes the fix.

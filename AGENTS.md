# AGENTS.md

Notes for AI coding agents working in this repo.

## What this is

Cockpit Tools — a Tauri 2 desktop app (package `cockpit-tools`) that manages accounts and
multi-instance launches for AI IDEs/CLIs (Codex, Antigravity, GitHub Copilot, Cursor, …).
React 19 + TypeScript frontend in `src/`; Rust workspace: `src-tauri` (Tauri app; lib crate is
`antigravity_cockpit_tools_lib`), `crates/cockpit-core` (shared logic for app + CLI),
`crates/cockpit-cli` (headless CLI). Tauri commands live in `src-tauri/src/commands/`,
registered in `lib.rs`.

## Builds

- **Never produce a release binary with plain `cargo build` / `cargo build --release`.** Tauri's
  macros keep `devUrl` (localhost:1420), so the binary opens a blank white window. Build via
  `npm run tauri build` (bare executable: `npm run tauri build -- --no-bundle`), which embeds
  `dist/`. See docs/TAURI_BUILD_GUIDE.md.
- `npm run tauri …` wraps `scripts/tauri.cjs`: version sync, cargo PATH discovery, Windows
  vcvars64, and macOS temp-DMG mount cleanup. Plain `npx tauri …` bypasses all of that.
- `npm run tauri:dev` auto-typechecks and starts with `COCKPIT_TOOLS_PROFILE=dev`: data in
  `~/.cockpit_tools_dev` (not `~/.cockpit_tools`; override `COCKPIT_TOOLS_DATA_DIR`), local API
  port 1456, config `src-tauri/tauri.dev.conf.json`. Vite dev server is port 1420 (strict).
- `vendor/tao` + `vendor/muda` are crates.io replacements via `[patch.crates-io]` (Windows
  backports). Read each crate's `COCKPIT-PATCH.md` before touching; remove the override when
  upstream Tauri includes the fix.
- Go proxy engine was removed; `sidecars/` holds only manifests. Engine binaries under
  `sidecars/*/bin` are untracked, prepared locally with `npm run proxy-engine:prepare`
  (env-var driven; the script never downloads).

## Checks

- `npm test` — all TS tests (`*.test.ts` under `src/` + `tests/`, via tsx --test). Subset:
  `npm test tests/codexSessionFilters.test.ts`.
- `*.test.cjs` files under `tests/` are standalone node:test files, not collected by `npm test`:
  `node --test tests/<file>.test.cjs`.
- `npm run typecheck` — `tsc --noEmit`; strict with `noUnusedLocals`/`noUnusedParameters`.
- Rust tests are stateful and must run single-threaded: app lib `cargo test --package
  cockpit-tools --lib` (preflight sets `RUST_TEST_THREADS=1`), core `npm run test:rust:core`
  (`--test-threads=1`). Pass `--locked` to cargo as CI does.
- Standalone Rust regression crates in `tests/*` are not workspace members (they compile single
  src-tauri modules): `cargo test --manifest-path tests/codex-profile-copy/Cargo.toml --locked --lib`.
- `npm run release:preflight` — full local gate: locales → modal sizing (`--strict`) → typecheck
  → build → `cargo check` → `cargo test --lib` (src-tauri). `--skip-*` only for debugging.

## i18n & UI

- All user-visible strings are translated: add new keys to every file in `src/locales/`
  (`en-US.json` is the baseline). `node scripts/check_locales.cjs` fails on key drift and
  untranslated reuse, including references collected from Rust code.
- Modal CSS rule (enforced by `node scripts/check_modal_sizing.cjs --strict`): at most one
  size-declaring class per modal root; override shared classes with `.modal.<own-class>`;
  modal bodies must not use `overflow: visible` or `max-height: none`. UX rules:
  docs/ui-design-guidelines.md.

## Release

`docs/release-process.md` is authoritative (including which local workspace releases run in).
- `package.json` version is the source of truth: run `npm run sync-version`, update both
  `CHANGELOG.md` and `CHANGELOG.zh-CN.md`, tag exactly `v<package.json.version>` (workflow
  hard-fails on mismatch).
- No `release:checksums` or `release:github-and-cask` npm scripts exist despite older docs;
  checksums and the Homebrew Cask update happen inside the release workflow.
- Local test DMG: `node scripts/release/package-local-dmg.cjs --app "target/release/bundle/macos/Cockpit Tools.app" --output <path>`.

## Conventions

- Commits: conventional style (`fix(macos-menu): …`); PRs target `main`.
- `crates/cockpit-core/src/modules/data_paths.rs`: upgrade by adding an alias, never by moving
  live profiles — canonical paths also identify Codex window storage.

# Changelog

English · [简体中文](CHANGELOG.zh-CN.md)

All notable changes to Cockpit Tools will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---
## [0.20.15] - 2026-03-30

### Added
- **A dedicated 2FA Manager page is now available in Classic sidebar navigation**: users can query Base32 secrets, view rolling OTP codes, save favorites, review recent history, and import/export saved records as JSON from one workspace.

### Changed
- **Codex multi-account local storage is now unified under `~/.antigravity_cockpit` with one-time migration from legacy paths**: existing `codex_accounts.json` and account detail files are copied to the new directory without overwriting newer files.
- **Grid-view batch selection is now consistent across account pages**: `Select All` is now shown in grid mode for platform account pages and shared suite views, including grouped-by-tag rendering.
- **2FA page labels and actions are now fully wired to i18n keys across locales**: navigation labels, confirmations, table headers, and action text no longer rely on hard-coded UI strings.
- **The log viewer modal footer now includes an explicit close action**: users can dismiss the dialog directly from the footer without relying on header controls.

---
## [0.20.14] - 2026-03-28

### Added
- **CodeBuddy CN and WorkBuddy now share one account workspace and check-in flow with synchronized capabilities**: both platforms now use the same account list/table rendering, check-in modal interactions, and parsing/normalization pipeline so account actions and quota display behavior stay consistent.
- **CodeBuddy CN now supports daily check-in end-to-end**: the account page and desktop command layer now include check-in API integration, status presentation, and in-context check-in dialog interactions.

### Changed
- **CodeBuddy CN quota presentation now uses a four-category model with unified aggregation logic**: quota data is reorganized into `base`, `activity`, `extra`, and `other` groups, and shared suite models now drive account-page and dashboard totals consistently.
- **Settings now complete CodeBuddy CN and WorkBuddy refresh controls in one loop**: quick settings and settings page now expose aligned refresh options for both platforms and reuse shared auto-refresh wiring.
- **Cloud Code quota requests now build metadata and User-Agent from detected official Antigravity installation details**: local quota fetching and onboarding now derive IDE version, platform, and client headers dynamically (including `x-goog-api-client`) instead of relying on hard-coded version/header values.

### Fixed
- **CodeBuddy instances and dashboard cards now resolve account type and quota aggregation more accurately**: instance rows now follow shared account-type mapping and dashboard cards no longer mix incorrect aggregates across providers.
- **Check-in i18n keys are now fully aligned across locales (including ar and zh-tw)**: missing keys are added and duplicated English fallbacks are removed to keep localized check-in UI complete.

---
## [0.20.13] - 2026-03-28

### Changed
- **Antigravity wakeup now aligns official Language Server startup flags by selected client version mode**: Wakeup Tasks and Account Verification now expose an `>=1.21.6 / <1.21.6` selector, the selection is persisted locally and synchronized to desktop runtime, and the wakeup gateway now appends `--random_port` only for `<1.21.6` mode to match older official client behavior.
- **Wakeup account pickers now support combined search + type/tag/group filtering with visible-scope batch selection**: task editing, manual tests, and account verification can all filter by account type, tags, and groups (including ungrouped), while “select all” now operates on the currently filtered result set.
- **Codex API Key credential input now validates field intent before save/import**: API Key values that look like URLs are rejected, Base URL must be a valid HTTP(S) URL, and duplicate API Key/Base URL values are blocked to prevent swapped-input mistakes.
- **Codex missing-path dialogs now support disabling launch-on-switch directly in place**: users can keep account switching/login-overwrite behavior while turning off automatic Codex app launch, and once disabled the missing-path prompt will no longer keep reappearing.

---
## [0.20.12] - 2026-03-27

### Changed
- **macOS tray interactions now align left-click and right-click behavior with native expectations while clearing stale menu highlight state**: left-click release now focuses and restores the main window, right-click press opens the tray context menu, and native menu teardown explicitly clears status-item highlight to avoid a stuck highlighted icon.
- **Antigravity account store persistence now keeps only minimal account snapshots and recovers gracefully when localStorage quota is exceeded**: persisted token fields are sanitized, quota snapshots exclude heavy model payloads, and quota overflow now auto-cleans legacy/new cache keys instead of repeatedly failing writes.

---
## [0.20.11] - 2026-03-27

### Added
- **Codex now includes a dedicated Session Manager for multi-instance thread sync and trash cleanup (thanks @GiZGY, PR #324)**: users can sync missing session threads across instances from one place and move selected sessions to Trash with per-session visibility and grouped workspace context.
- **Codex wakeup manual tests can now be cancelled while running**: test runs now carry a cancellation scope, the desktop wakeup execution can terminate in-flight Codex CLI processes, and the execution-results dialog supports explicit in-run cancellation.

### Changed
- **Homebrew cask metadata has been refreshed to keep packaged distribution in sync with the latest release assets**: the cask formula has been aligned with the current published binaries and checksums.

---
## [0.20.10] - 2026-03-27

### Added
- **Antigravity wakeup manual tests can now be cancelled directly from the active test dialog**: each test run now carries a cancellation scope through the desktop wakeup pipeline, so cancelling stops in-flight wakeup requests cleanly and shows a dedicated cancellation notice instead of waiting for every request to finish.

### Changed
- **Classic sidebar navigation is now flatter and keeps existing layout preferences through a unified local-store migration**: classic mode no longer depends on expandable grouped sections, remaining entries appear directly in `More`, the collapse handle moves with transform-based animation, and legacy sidebar preference keys are migrated into the new persisted store on upgrade.
- **Antigravity account cache persistence is now consolidated into a unified persisted store with legacy-key migration**: cached account lists and current-account snapshots are rehydrated from the new store while older local keys are migrated and cleaned up automatically.

---
## [0.20.9] - 2026-03-25

### Added
- **Added a classic sidebar layout mode with full platform navigation, collapsible sidebar width, grouped-entry expansion, and an in-sidebar logs entry**: users can now switch from the compact original rail to a full-height classic navigation shell that supports inline group children and adaptive scaling in constrained window heights.

### Changed
- **Sidebar layout configuration now supports mode-specific behavior across Settings and platform layout management**: Settings now provides an `Original / Classic` layout selector, first-time entry into classic mode syncs sidebar entries from dashboard visibility, and the platform layout modal now allows unlimited sidebar selections in classic mode while keeping the original mode limit.
- **Antigravity account quota display groups are now fixed to built-in model families (Claude / Gemini Pro / Gemini Flash)**: account page rendering now uses predefined display groups directly, and no longer depends on manual group-settings configuration.
- **Documentation now includes Arch Linux AUR installation paths**: README and README.en.md now document both source-built (`cockpit-tools`) and prebuilt (`cockpit-tools-bin`) AUR packages.

---
## [0.20.8] - 2026-03-24

### Fixed
- **macOS shell-launched proxy environments now remain effective when Cockpit's in-app global proxy is not explicitly enabled**: app startup and config saves now restore the proxy variables inherited at launch instead of clearing them outright, so workflows such as `export http_proxy=... && open -a 'Cockpit Tools'` continue to work unless users intentionally override proxy settings inside the app.

---
## [0.20.7] - 2026-03-24

### Changed
- **Floating account cards now stay synchronized with account imports, deletions, OAuth completions, and current-account switches across windows**: provider pages and account stores now emit shared account-sync events, so floating cards refresh immediately after account management actions instead of waiting for manual reloads or window refocus, while instance-bound floating cards keep their bound account view.
- **Windsurf official quota panels on the account page now render daily and weekly progress bars with separate low and critical warning colors**: quota items that come from Windsurf's official plan snapshot now use the shared quota-progress styling instead of showing percentage text only, making the remaining-risk state easier to scan at a glance.

### Fixed
- **Current-account detection now follows real local state more tightly after sync, deletion, switching, and empty-list transitions**: provider stores clear stale current-account ids when no accounts remain, current-account changes are propagated immediately after sync/delete/switch flows, and instance-bound floating cards no longer blank out just because the platform cannot resolve a separate current account at that moment.
- **Windsurf quota-billed accounts now keep quota mode and convert official remaining-percent fields into used-percent displays consistently across the account page, tray, macOS native menu, and diagnostic report**: quota views now treat `dailyQuotaRemainingPercent` / `weeklyQuotaRemainingPercent` as remaining quota and fall back to exhausted usage when quota billing omits those fields, so quota-backed accounts no longer slip into credit-mode presentation or invert their usage percentage.

---
## [0.20.6] - 2026-03-24

### Changed
- **Codex wakeup account selection now shows primary and secondary quota badges inline**: wakeup account chips display two compact quota indicators beside the masked account context, so users can compare standard quota state before selecting accounts without opening the full account view.

### Fixed
- **Cross-platform desktop Rust builds now keep Codex and Qoder helper modules aligned with target-specific compilation rules**: Codex CLI install hints now compile cleanly on both macOS and non-macOS targets, and Qoder OAuth path utilities are no longer gated behind a Unix-only import.

---
## [0.20.5] - 2026-03-24

### Fixed
- **Windsurf quota-billed accounts now show the official used-percent values consistently across the account page, tray, macOS native menu, and diagnostic report**: daily and weekly quota usage is now read directly from the upstream usage fields instead of being inverted as if it were remaining percentage, preventing quota progress from being displayed backwards or incorrectly pinned to exhaustion.

---
## [0.20.4] - 2026-03-24

### Added
- **Codex wakeup now supports model presets and per-task reasoning-effort selection end-to-end**: wakeup tasks and manual tests can pick a managed model preset plus reasoning effort, execution records now store the model metadata, and wakeup runs pass `model` / `model_reasoning_effort` directly into Codex CLI execution.
- **Codex wakeup scheduling now supports quota-reset triggers with window selection**: tasks can run after `primary_window`, `secondary_window`, or either reset window, and the scheduler computes due and next runs from real account quota reset timestamps.

### Changed
- **Quota-reset wakeup tasks now enforce a fast Codex quota refresh cadence**: when at least one enabled quota-reset task exists, Codex auto refresh is adjusted to every 2 minutes so reset-trigger detection stays timely.
- **Desktop updater now supports reminder opt-out and per-version skip**: settings include an update reminder toggle, users can skip a detected version from the update dialog, skipped versions are ignored by subsequent checks, and the sidebar quick-update entry follows the reminder setting while preserving in-progress or ready states.
- **Account page view mode persistence is now unified across providers, including the new Codex compact view**: Codex overview adds a compact layout mode, and provider pages persist list/grid preferences with platform-scoped local storage keys.
- **Codex wakeup task summaries and execution details now mask displayed account emails**: task cards and execution result rows now hide full email addresses while preserving account context text and selected model metadata.
- **The floating account card window now disables the native window shadow**: the transparent desktop floating-card window is configured without the system shadow layer.

---
## [0.20.3] - 2026-03-24

### Fixed
- **Desktop update prompts now stay on a single app-controlled check flow instead of being re-checked inside the popup**: startup and manual checks now reuse the same updater result, detected updates are no longer lost because the dialog performs a second `check()`, silent downloads reopen the same dialog in the ready-to-restart state, and the app performs one startup check followed by hourly polling while it remains open.

### Changed
- **Codex wakeup now keeps a managed per-account `CODEX_HOME` instead of creating a temporary profile for every run**: each account now reuses a stable local wakeup home, `auth.json` is rewritten atomically before execution, and wakeup runs no longer create and delete a fresh temporary profile directory on every trigger.
- **Windows process probing now uses a single inline PowerShell path without temporary script fallback**: Windows detection and launch helpers no longer write transient `.ps1` files or invoke `ExecutionPolicy Bypass` as a fallback when inline PowerShell execution fails.

---
## [0.20.2] - 2026-03-23

### Fixed
- **Bundled macOS Codex wakeup builds now detect Homebrew-installed Codex CLI and its Node runtime without depending on the terminal PATH**: the desktop app now augments packaged GUI detection with standard macOS CLI install directories, resolves the `codex` launcher and required `node` interpreter through the same runtime search path, and prevents released `.app` builds from falsely reporting that Codex CLI is not installed when it is available under `/opt/homebrew` or `/usr/local`.
- **Windows Codex wakeup CLI checks no longer flash a black console window during runtime probing**: Codex CLI version probing and wakeup command launches now apply the hidden-window process flags consistently, so packaged desktop builds no longer briefly open a console window when checking CLI availability or running a wakeup command.

### Changed
- **Codex wakeup CLI probing now writes targeted desktop logs for packaged-app diagnosis**: CLI rechecks, version probing, runtime resolution, and wakeup execution now emit `[CodexWakeup][CLI]` log lines with the resolved search directories, launcher path, Node path, and process failure output so packaged-app environment issues can be diagnosed directly from `app.log`.

---
## [0.20.1] - 2026-03-23

### Added
- **Codex wakeup tasks now include an always-available execution details view**: each task card adds a dedicated details icon that opens the same execution-results dialog used by manual tests, so users can inspect queued accounts before a run starts and keep watching the same panel when a scheduled run begins.

### Fixed
- **Codex wakeup task cards now guard manual runs and count trigger history by task run instead of per-account records**: clicking the manual-run action now requires confirmation before immediately waking accounts, and the history badge reflects grouped task/test executions instead of inflated per-account totals.
- **Release asset publishing now keeps updater bundles in a single GitHub release before merged metadata is rebuilt**: the release workflow now creates one draft release up front and uploads matrix artifacts by `releaseId`, preventing split draft releases from breaking merged `latest.json` and `SHA256SUMS.txt` generation across macOS, Windows, and Linux.

---
## [0.20.0] - 2026-03-23

### Added
- **Codex now includes a dedicated Wakeup Tasks workspace for OAuth accounts**: the Codex page adds a `Wakeup Tasks` tab where users can create daily/weekly/interval jobs, check Codex CLI availability and install hints, run manual wakeup tests, preview upcoming runs, and review per-account execution history with live progress.

### Changed
- **Codex wakeup execution is now backed by persisted desktop scheduling instead of page-local state only**: task definitions and run history are saved locally, the desktop app starts a background scheduler on launch, manual task runs refresh Codex account data after completion, and Windows wakeup runs launch the Codex CLI without showing a console window.

### Fixed
- **Modal failure feedback is now kept inside the active dialog across account, wakeup, verification, fingerprint, and instance flows**: delete/save/bind errors now keep the modal open, render in a dedicated in-modal error area, auto-scroll to the failing section, and clear stale errors before the next submit or when the modal closes.

---
## [0.19.2] - 2026-03-23

### Fixed
- **Windows floating-card action buttons no longer get swallowed by window dragging hit-tests**: clicking the floating card's close, pin, and account-navigation controls now consistently triggers the intended action instead of being misclassified as a drag start when the pointer lands on SVG icon nodes.

---
## [0.19.1] - 2026-03-23

### Fixed
- **Bundled macOS menu-bar builds no longer crash when opening the native Swift tray menu**: native menu provider icons are now bundled into the main app resources and loaded from the packaged app bundle, so clicking the menu-bar entry in the released `.app` no longer triggers a missing-resource assertion in `Bundle.module`.

---
## [0.19.0] - 2026-03-23

### Added
- **Added a dedicated floating account card window with current/recommended account preview, quick switch, pinning, and instance-bound overlays**: the app can now show a compact floating card on startup or on demand from Settings/Tray, supports per-instance floating cards bound to managed instances, remembers window position, and provides close-confirm guidance plus direct navigation back to the main page.
- **macOS tray interaction now includes a native Swift popover menu for account overviews and switching**: the menu-bar entry now opens a native provider switcher and account card panel backed by synced platform snapshots, with direct actions for switching accounts, opening details, and reopening the main window.

### Changed
- **Provider current-account resolution is now unified around real local bindings and runtime state instead of browser-only guesses**: Cursor, Gemini, Kiro, Windsurf, CodeBuddy, CodeBuddy CN, Qoder, Trae, WorkBuddy, and Zed now resolve their current account through backend store logic, and dashboard/account pages/tray/floating card all reuse the same current-account state instead of diverging through `localStorage`-only fallbacks.
- **Platform account export now follows the accounts currently visible on the page after filtering**: bulk export on provider pages now exports the filtered account list shown in the current view, and when some visible accounts are selected it exports only those visible selections instead of including hidden selections from outside the current filter scope.
- **Current-account refresh now propagates to runtime surfaces immediately after sync or switch**: Antigravity client sync now refreshes the tray after reading local client state, provider stores refresh current-account state after account fetch/switch, and shared account pages no longer hardcode the `Available AI Credits` label outside locale keys.

---
## [0.18.3] - 2026-03-22

### Added
- **Provider account indexes now auto-repair from local detail files when the index is missing, empty, or corrupted**: Codex, Cursor, GitHub Copilot, Gemini, Kiro, Qoder, Trae, Windsurf, CodeBuddy, CodeBuddy CN, WorkBuddy, and Zed now back up the broken index, rescan per-account detail JSON files, rebuild the account list in recency order, and write the recovered index back to disk before the page continues loading.

### Changed
- **Account pages now show unrecoverable local file corruption more directly**: shared provider account pages now reuse the corrupted-file parser so users see the damaged file name instead of only a generic error when automatic repair cannot recover the index.

---
## [0.18.2] - 2026-03-22

### Changed
- **Account groups now follow the real local persistence path instead of browser-only storage**: group definitions are now saved in `~/.antigravity_cockpit/account_groups.json`, legacy `localStorage` data is migrated on first load, and moving accounts between groups updates the persisted group file rather than only changing frontend state.
- **Grouped account management on the account page is now denser and easier to operate**: list and compact views surface group rows inline, selected accounts inside a group can be moved to another group or removed directly from the breadcrumb actions, and the group member picker reuses the same tier/tag filters as the main page.

### Added
- **Account tag editing now supports per-account notes saved with the account record**: the tag modal can edit up to 200 characters of notes alongside tags, and the new notes field is written through the account persistence layer so it stays available across restarts.

---
## [0.18.1] - 2026-03-22

### Changed
- **Zed account presentation is now unified across the account page, dashboard, and tray**: Zed plan labels now strip the `zed_` prefix and display the remaining raw identifier in uppercase, dashboard badges reuse the same visual treatment as the main account page, and `Edit Predictions` summaries keep single-line `used / total` output in compact cards.
- **Zed menu-bar behavior now follows the saved platform layout and desktop-derived status fields consistently**: disabling Zed in the tray layout no longer gets reverted by legacy migration, the side-menu/platform-layout entry is available again, and tray summaries now match the main page by showing `Edit Predictions` plus overdue status instead of stale token-spend rows.

---
## [0.18.0] - 2026-03-22

### Added
- **Zed account management is now supported end-to-end with official native-app OAuth, JSON import, local current-session import, and real credential apply-back into the official client**: added a dedicated Zed account page, local account storage/indexing, current-session runtime controls, and account apply/restart behavior aligned with the official desktop credential slot.
- **Zed is now integrated into dashboard summaries, global account transfer bundles, and platform settings**: dashboard cards, transfer export/import, launch-path configuration, auto-refresh cadence, and quota-alert settings now include Zed.

### Changed
- **Zed quota display now follows the desktop client's actual account API instead of browser billing pages**: the page now focuses on `Edit Predictions` and overdue-invoice status from `/client/users/me`, supports importing the currently signed-in local account, and logs the raw refresh payload for diagnostics.
- **Zed platform presentation has been tightened for the current rollout**: the app now uses the official Zed application icon, the add-account modal is aligned with the shared `OAuth / Token / Import` structure, and the side-menu entry is hidden while menu maintenance remains paused.

---
## [0.17.8] - 2026-03-21

### Fixed
- **Codex API Key accounts now write the official `openai_base_url` key into `~/.codex/config.toml` when a custom base URL is configured**: account switching and local injection no longer persist the incorrect `base_url` key, so Codex can read the configured upstream API endpoint correctly.

---
## [0.17.7] - 2026-03-21

### Changed
- **Windsurf usage summaries in both dashboard and tray now stay aligned with the official quota vs. credits billing model**: quota-billed accounts now show daily quota usage, weekly quota usage, and extra usage balance, while credits-billed accounts keep the credits-left breakdown without being misclassified by enum-style billing strategy values.
- **Dashboard account cards now render platform-specific quota structures more faithfully across multiple providers**: Kiro, Gemini, CodeBuddy, CodeBuddy CN, Qoder, Trae, and WorkBuddy cards now show the correct remaining/used summaries, reset or expiry timing, and related status details through the shared account presentation layer.

---
## [0.17.6] - 2026-03-20

### Changed
- **Windsurf usage-mode detection now follows the official billing-strategy enums end-to-end**: the account page now normalizes raw `BILLING_STRATEGY_*` values before deciding between quota and credits, preventing official quota accounts from being misclassified when Windsurf stores enum-style strategy strings.
- **Windsurf quota panels now keep the official three-field summary visible**: quota-billed accounts now always render daily quota usage, weekly quota usage, and extra usage balance, defaulting the extra-balance row to `$0.00` when the local snapshot does not include an upstream balance value.

---
## [0.17.5] - 2026-03-20

### Changed
- **Windsurf account usage now follows the official billing mode presentation**: quota-billed accounts now show daily/weekly quota usage, reset times, and extra usage balance, while credits-billed accounts show combined credits left with prompt/add-on breakdown.
- **Manual help entry is now unified across dashboard and overview headers**: dashboard and platform overview headers now reuse one shared help icon button with consistent sizing, hover feedback, and navigation behavior.

---
## [0.17.4] - 2026-03-20

### Changed
- **Account plan filtering now supports multi-select across major account pages**: Accounts, Codex, Cursor, Gemini, GitHub Copilot, Kiro, Qoder, Trae, Windsurf, CodeBuddy, CodeBuddy CN, and WorkBuddy pages now allow selecting multiple plan/status types in one pass.
- **Plan filter interaction is now unified through a shared dropdown component**: introduced reusable multi-select filter UI with selected-count indicator, one-click clear, and consistent filtering behavior across provider pages.

---
## [0.17.3] - 2026-03-20

### Added
- **Desktop now includes a built-in live log viewer for runtime diagnostics**: added a floating log entry button in the main window, with latest-log tail view, auto refresh, line-limit control, clear/copy content, copy log path, and one-click open log directory.
- **Backend log commands now support latest-file snapshot and bounded tail reads**: added latest `app.log*` discovery, tail line clamping, and cross-platform log-directory open commands for frontend diagnostics.

### Changed
- **CodeBuddy and WorkBuddy usage-state rendering is now unified with explicit abnormal details**: account cards/tables now show a normalized abnormal state with a detail modal and masked account context, and CodeBuddy/WorkBuddy instance quota previews now use shared dosage-notify renderers for consistent output.

---
## [0.17.2] - 2026-03-20

### Changed
- **Windows Codex launch now aligns to the Microsoft Store system registration entry by default**: startup now prefers Store AppUserModelId (`shell:AppsFolder`) instead of directly executing `WindowsApps/.../Codex.exe`, with exe-path fallback only when Store entry launch is unavailable.
- **Windows Codex launch precheck now validates Store installation availability first**: launch-path readiness now succeeds on detected Store registration entry and only falls back to executable-path validation when needed.

### Added
- **Windows Codex launch logs now explicitly expose startup strategy selection**: launch logs now print whether `system-store-entry` or `exe-path` was used, including matched app id/path and resolved pid for troubleshooting.

---
## [0.17.1] - 2026-03-20

### Added
- **Network settings now support a managed global proxy for launched platform processes**: added `global_proxy_enabled`/`global_proxy_url`/`global_proxy_no_proxy`, with env injection into Cockpit-launched platform apps and instance start flows across macOS/Linux/Windows.
- **GitHub Copilot switching now supports OpenCode sync controls and launch toggle**: added quick-settings/general-config options to control GitHub Copilot app auto-launch, OpenCode auth overwrite, and OpenCode restart after switch.

### Changed
- **OpenCode restart toggles now follow auth-overwrite dependency in both Settings and Quick Settings**: when auth overwrite is turned off, related restart toggles are now automatically turned off for both Codex and GitHub Copilot switch flows.

---
## [0.17.0] - 2026-03-19

### Added
- **Codex account switching now supports an optional OpenClaw login-overwrite toggle**: Settings and Quick Settings now expose `openclaw_auth_overwrite_on_switch`; when disabled, only Codex is switched and OpenClaw keeps its current login state.

### Changed
- **Codex-to-OpenClaw credential sync now writes and verifies `openai-codex:default` end-to-end**: switch flow now updates OpenClaw `auth-profiles.json`, cleans stale `openai-codex:*` profiles, syncs candidate paths, and validates account/email/expiry consistency with Codex credentials.
- **macOS Codex switching now updates keychain `Codex Auth` alongside `auth.json`**: keeps external-cli/OpenClaw credential reads aligned with the active Codex account.
- **OpenClaw post-sync runtime refresh now retries reload/restart for faster effect**: after sync it attempts `secrets reload` and `gateway restart`, then performs one retry and logs actionable diagnostics if consistency checks still fail.

---
## [0.16.3] - 2026-03-19

### Added
- **Kiro Enterprise/IdC local-import accounts now support AWS IAM Identity Center OIDC refresh path**: the refresh flow now prefers OIDC token refresh with local IdC context and falls back to Kiro `refreshToken` endpoint for compatibility.

### Changed
- **Kiro plan/tier badge text now follows raw subscription values first**: account and instance views now prioritize `plan_name`/`plan_tier`/usage raw labels to stay aligned with official client naming.
- **Kiro import parser now keeps Enterprise refresh context fields**: JSON import now carries `authMethod`, `login_option`, `startUrl`, `client_secret`, and related IdC fields for post-import refresh continuity.
- **Kiro flow-notice localization now reflects actual Enterprise refresh network scope across all supported languages**: wording now explicitly covers AWS OIDC calls and required OIDC auth fields.

---
## [0.16.2] - 2026-03-19

### Added
- **Codex API Key accounts now support custom Base URL end-to-end with local persistence**: API Key add/import/switch flows now read and write `base_url` (including `config.toml`) and keep account metadata synced with local auth files.
- **Codex API Key accounts now support in-place credential editing**: account cards/tables add an edit action for API Key + Base URL, and backend updates account id/index, current-account mapping, and instance bindings in one operation.

### Changed
- **Codex quota-error cards/tables now provide direct OAuth reauthorize action for token-invalid scenarios**: `401`/`token_invalidated`-like errors can jump back into OAuth authorization from the quota error area.

### Fixed
- **Local-import account lists now refresh more reliably across pages**: shared provider, Codex, Qoder, and generic accounts pages add a short delayed refetch after import to avoid transient index-write lag.
- **Trae refresh diagnostics are now richer for non-JSON upstream responses**: parse errors now include HTTP status, key response headers, and a safe body preview to speed up troubleshooting.

---
## [0.16.1] - 2026-03-18

### Added
- **Codex now supports API Key account onboarding and import paths end-to-end**: added dedicated API Key add flow, `auth_mode=apikey` persistence in `~/.codex/auth.json`, and JSON/local import compatibility for API Key records.

### Changed
- **Codex account cards/tables now adapt to API Key account behavior**: API Key accounts support inline rename, show masked key metadata, hide unsupported quota-refresh actions, and provide a direct link to OpenAI Usage.
- **Codex backend refresh/injection paths now skip OAuth-only operations for API Key accounts**: profile/quota refresh and refresh-all scheduling bypass API Key accounts while keeping account switching and auth-file write behavior aligned with local storage rules.
- **Codex instance account picker now uses the same presentation display name logic as the account list**: renamed API Key accounts are shown consistently in instance selection.

---
## [0.16.0] - 2026-03-18

### Added
- **Cross-platform account transfer center in Settings**: Added one-click export/import for all platforms with a unified JSON bundle schema, platform-level import progress, and modal/file workflows.
- **Platform grouping and quick-switch UX across core surfaces**: Added editable platform groups (name, icon, default child, child-level metadata), group switcher in headers, grouped cards on dashboard, and grouped entry rendering in side navigation and layout modal.
- **Custom group icon library with local persistence**: Added icon upload, reuse, and cleanup for group/child icons in platform layout configuration.

### Changed
- **Tray layout model now supports ordered entries plus platform groups**: tray persistence now stores `orderedEntryIds` + `platformGroups`, and tray menu rendering now understands grouped entries while keeping manual ordering and visibility controls.
- **macOS app launch flow now aligns around LaunchServices `open -n -a` with PID probing for isolated instances**: Antigravity/Codex/VS Code/CodeBuddy/CodeBuddy CN/WorkBuddy plus Qoder/Trae/Cursor/Kiro/Windsurf start paths now use consistent launch semantics and post-launch PID matching for target profiles.
- **Account refresh reliability improved with delayed retry across providers**: Antigravity quota refresh and multiple provider token/profile/quota refresh paths now perform one delayed retry with unified logs before surfacing failure.
- **Codex OAuth add-account flow now supports in-place token-exchange retry**: OAuth error state now exposes a retry action for token exchange without restarting the full authorization flow.
- **Settings, dashboard, and navigation visuals were updated for grouped-platform operations**: added new layout/modal/switcher/transfer styles and supporting locale keys across all supported languages.

---
## [0.15.1] - 2026-03-16

### Changed
- **Codex auto-switch and quota alerts now support independent `primary_window`/`secondary_window` thresholds end-to-end**: backend config normalization, candidate selection, cooldown keys, and post-refresh checks now evaluate dual thresholds and can switch accounts before alerting when a better candidate exists.
- **Codex Quick Settings now expose dual-window controls for auto-switch and quota alerts**: added dedicated percentage inputs for `primary_window` and `secondary_window`, plus combined OR-condition hints and modal threshold display.
- **Codex quota refresh scheduling now includes a 60-second current-account refresh when auto-switch or quota alerts are enabled**: improves trigger timeliness without changing existing full-refresh interval behavior.
- **Default-instance launches triggered by switching now pass saved extra args for Antigravity and Codex**: switch flows and the default Codex instance start path now reuse configured `extra_args` instead of dropping them.
- **Codex refresh flows now always update current-account state after refresh**: manual and batch refresh paths now hydrate both account list and current account for consistent UI state.
- **Windows main window default width increased to 1250**: provides more horizontal space for account and quick-settings content.

---
## [0.15.0] - 2026-03-15

### Added
- **WorkBuddy platform full integration with account sync and instance management**: Added WorkBuddy backend/frontend modules, OAuth/Token/JSON/local import flows, account switching via local credential injection, dashboard/settings/quick-settings/tray integration, and bidirectional account sync with CodeBuddy CN.
- **Token-protected HTTP usage report service with optional HTML rendering**: Added configurable `/report` endpoint (`report_enabled` + port + token) that aggregates multi-platform quota summaries and supports raw Markdown/YAML output plus `render=true` HTML view for browser inspection.

### Changed
- **CodeBuddy, CodeBuddy CN, and WorkBuddy runtime flows are now structurally aligned**: Moved cross-platform sync/injection paths into WorkBuddy domain modules, unified command registration, and reduced duplicated maintenance paths.
- **Account injection/startup reliability and operator guidance are improved**: CodeBuddy/CodeBuddy CN/WorkBuddy instance injection now includes post-write verification and clearer “sign in manually first” guidance for keychain/state-db failure cases.
- **Rendered report readability and staleness metadata are enhanced**: Report pages now group rows more clearly, show human-readable local timestamps, and expose delayed-refresh/stale-data notes directly in report metadata.

### Contributors
- `PR #213` by `@lihongjing-2023`: WorkBuddy platform integration and account sync.
- `PR #212` by `@lovitus`: tokenized web report service and rendered report improvements.

---
## [0.14.5] - 2026-03-15

### Changed
- **Startup work after upgrade is now lighter and more staggered**: boot now renders immediately with built-in `en`/`zh-CN` resources instead of blocking first paint on async language loading, auto-refresh timer setup is deferred, dashboard platform prefetch is batched, and frontend vendor/update chunks are split more aggressively to reduce first-launch stalls after larger upgrades.

### Fixed
- **Codex account refresh now recovers from backend-invalidated auth tokens**: quota refresh and account-profile checks now detect `401 token_invalidated`, force one refresh-token exchange, persist the rotated tokens, and retry the official ChatGPT usage/account-check endpoints instead of surfacing a false “sign in again” failure while the same account can still be used in the official client.
- **Codex default instance launch on macOS no longer reuses another running isolated instance**: when a non-default Codex instance is already open, starting the default instance now forces a fresh LaunchServices app instance so the default profile opens instead of just focusing the existing isolated window.

---
## [0.14.4] - 2026-03-14

### Fixed
- **Cursor account list now self-heals from local account files when index is missing or corrupted**: listing now recovers accounts from `cursor_accounts/*.json`, rewrites `cursor_accounts.json`, and keeps accounts/tags visible instead of showing an empty list when only the index file is damaged.
- **Cursor account list now uses the same index lock as write operations**: `list_accounts` is serialized with import/delete/update paths to reduce race windows during concurrent index access.

---
## [0.14.3] - 2026-03-14

### Fixed
- **OAuth add-account dialogs now reliably re-open for repeated sign-ins across platforms**: after OAuth success, Codex and shared provider OAuth flows now clear pending-session/UI residue (auth URL, timeout/polling/manual-callback state) immediately and on modal close/tab change, preventing subsequent logins from stalling after `OAuth start`.

---
## [0.14.2] - 2026-03-14

### Fixed
- **GitHub Copilot switching now falls back to VS Code Insiders local storage automatically**: when the standard VS Code `Code` user-data path is missing, default-instance and token-injection flows now probe `Code - Insiders` for `state.vscdb`, Windows `Local State`, and VS Code safe-storage credentials, restoring switching for Insiders-only installs.

---
## [0.14.1] - 2026-03-14

### Added
- **CodeBuddy/CodeBuddy CN resource-package quota is now shown directly in app**: account cards and tables now render per-package quota amount, progress, and refresh/expiry time (including extra credits) without requiring web-only viewing.
- **CodeBuddy CN Token import now hydrates quota metadata immediately**: Token import now pulls dosage/payment/user-resource payloads and persists them into quota and usage fields during account creation.

### Changed
- **Quota refresh now uses IDE access tokens instead of Cookie binding**: backend refresh now calls `/v2/billing/meter/get-user-resource` with Bearer token and identity headers, and writes refresh errors into account state for UI visibility.
- **Legacy manual quota-binding flow is removed across backend and frontend**: removed cURL replay binding, binding-clear commands, and related service/type paths for both CodeBuddy and CodeBuddy CN.
- **Localization copy is aligned with the new quota model across all locales**: removed obsolete Cookie-binding flow text, updated network-scope wording to resource-package quota refresh, and added package-title keys for base/activity packages in all supported languages.
- **HTTP decoding compatibility for quota APIs is expanded**: `reqwest` now enables `brotli`/`deflate`/`zstd` features to handle compressed billing responses more reliably.

### Fixed
- **Quota summary and recommendation no longer depend on legacy binding state**: resource summary no longer returns `null` when `quota_binding` is absent, preventing fallback-only recommendation behavior after token refresh.

---
## [0.14.0] - 2026-03-13

### Added
- **CodeBuddy CN platform full integration**: Added CodeBuddy CN models, commands, modules, OAuth flow, account pages, services, stores, icons, navigation, dashboard/tray wiring, and multi-instance management support.
- **CodeBuddy CN account lifecycle support**: Added browser-based OAuth login, Token/JSON import, local client import, account switching with local credential injection, tag management, and account export.
- **Manual OAuth callback URL input**: OAuth flows that rely on a local callback port now support manual callback URL input when automatic callback capture is unavailable, improving authorization success in restricted network environments.

### Changed
- **CodeBuddy/CodeBuddy CN quota display simplified**: Quota information is now viewed on the web page; removed complex in-app quota query form for a cleaner account page experience.
- **Shared runtime surfaces now cover eleven platforms**: Dashboard, tray, settings, quick settings, auto-refresh scheduling, quota-alert preferences, navigation, and README/docs now include CodeBuddy CN consistently.

### Fixed
- **Qoder import now refreshes account list**: JSON and Token import on Qoder platform now correctly refresh account data after successful import, fixing display issues where imported accounts were not shown immediately.
- **Local import now refreshes tray summaries across multiple platforms**: Antigravity, Codex, Cursor, Kiro, Windsurf, Trae, and Qoder now update the tray menu immediately after successful local import, preventing shared-runtime summaries from staying stale after import.

---
## [0.13.0] - 2026-03-12

### Added
- **Qoder platform full integration across backend and frontend**: Added Qoder models, commands, modules, official CLI device-login flow, local/JSON import, account pages, services, stores, icons, navigation, dashboard/tray wiring, and raw-plan/quota presentation.
- **Qoder account switching and multi-instance management**: Added Qoder credential injection, default-instance binding, isolated multi-instance profiles, start/stop/open-window/close-all controls, and launch-path detection for macOS, Windows, and Linux.
- **Trae platform full integration across backend and frontend**: Added Trae models, commands, modules, OAuth flow, local/JSON import, account pages, services, stores, icons, navigation, dashboard/tray wiring, and plan/usage presentation.
- **Trae account switching and multi-instance management**: Added Trae local auth write-back using the client's actual on-disk rules, default-instance binding, isolated multi-instance profiles, start/stop/open-window/close-all controls, and launch-path detection for macOS, Windows, and Linux.

### Changed
- **Shared runtime surfaces now cover ten platforms**: Dashboard, tray, settings, quick settings, auto-refresh scheduling, quota-alert preferences, navigation, and README/docs now include Qoder and Trae consistently.
- **Settings now expose Qoder/Trae path and quota controls**: General settings now cover Qoder/Trae auto-refresh, launch paths, and quota alerts in one place.
- **Gemini platform wording is now aligned as Gemini Cli**: Shared navigation, settings, and account-management labels now consistently use `Gemini Cli`.

### Fixed
- **Pending OAuth sessions are now cancelled when dialogs or pages close**: Provider OAuth flows now cancel in-flight authorization sessions on modal close, tab switch, or page unload to avoid stale pending sessions.
- **Windows updater now keeps installer type consistent to avoid duplicate desktop shortcuts**: Windows update checks now pass an explicit updater target based on the current bundle type (`windows-*-nsis` / `windows-*-msi`), and merged `latest.json` now points the `windows-x86_64` fallback to NSIS to prevent installer-type drift from recreating desktop shortcuts during update.

---
## [0.12.3] - 2026-03-11

### Fixed
- **macOS permission prompts no longer attribute to Cockpit Tools**: All IDE launches (Codex, VS Code, CodeBuddy) on macOS now use `open -a` via LaunchServices instead of direct binary execution, so macOS TCC permission dialogs (e.g. Downloads folder access) correctly attribute to the launched IDE rather than Cockpit Tools. Multi-instance PID tracking is preserved through post-launch process polling.

---
## [0.12.2] - 2026-03-11

### Added
- **Linux package installs now support managed in-app updates**: Added `.deb`/`.rpm` runtime detection, signed package download, progress reporting, and privileged install flow so Linux package-manager installs can complete updates directly in Cockpit.
- **Antigravity accounts now support local account groups**: Added local folder-style account groups on the Antigravity accounts page, including create/rename/delete, batch add/remove, grouped browsing, and per-group quota refresh.

### Changed
- **Windsurf plan presentation now recognizes more official tiers**: Windsurf account cards, badges, and filters now resolve Trial, Teams, Teams Ultimate, and Pro Ultimate labels from remote plan data and teams-tier metadata.
- **Linux updater behavior now matches package-managed installs**: Background silent download is skipped for managed `.deb`/`.rpm` installs, and the sidebar/update dialog now shows authorization and installation progress states during one-click update.
- **Quota alert native notifications now follow the selected UI language**: Backend notification text now resolves from locale keys and covers Codex, GitHub Copilot, Windsurf, Kiro, Cursor, Gemini, and CodeBuddy consistently.

### Fixed
- **Wakeup task creation/test now checks runtime readiness first**: Opening “new task” and “test task” now stops early and reuses the existing runtime-path guidance when the wakeup runtime is not configured.
- **Settings and recovery dialogs now surface action failures inline**: Quick Settings path/config errors, file-corruption “open folder” failures, and global modal action failures are now shown in the UI instead of only logging to console.
- **macOS quota alerts no longer keep a click-wait notification loop alive**: Native quota notifications now use fire-and-forget delivery to avoid unnecessary background energy usage after notification delivery.

---
## [0.12.1] - 2026-03-10

### Added
- **Codex account profile hydration from official account-check endpoint**: Added a `refresh_codex_account_profile` backend/frontend flow to fetch and persist `account_name` and `account_structure`.
- **Automatic profile hydration for team-like Codex accounts**: Added store-level background hydration for accounts missing structure/name metadata, with in-flight de-duplication and a 5-minute retry interval.

### Changed
- **Codex account cards and tables now display account context**: account rows now show “Personal account” or hydrated team/workspace names based on structure, plan type, and workspace metadata.
- **Codex instance quota preview now follows Code Review visibility preference**: when Code Review quota is hidden in preferences, instance-page badges, search text, and quota preview now hide it consistently.

---
## [0.12.0] - 2026-03-10

### Added
- **CodeBuddy platform full integration across backend and frontend**: Added CodeBuddy models, commands, modules, OAuth flow, account pages, services, stores, icons, navigation, dashboard wiring, and shared platform metadata.
- **CodeBuddy account lifecycle support**: Added browser-based OAuth login, Token/JSON import, quota query and binding, cycle/resource/extra-credit presentation, tag editing, bulk actions, account export, and local credential injection for account switching.
- **CodeBuddy multi-instance management**: Added CodeBuddy instance store and commands with isolated user-data directories, account binding, instance create/update/delete, start/stop, open-window, and close-all controls.
- **CodeBuddy quota binding supports full cURL replay**: Added a full `Copy as cURL (bash)` workflow for `get-user-resource`, replaying the original request (method/headers/body) to improve binding accuracy and persist normalized quota binding parameters.

### Changed
- **CodeBuddy is now integrated into shared runtime surfaces**: Added CodeBuddy app-path detection, auto-refresh interval, quota-alert settings, Quick Settings, tray summaries, and global refresh scheduling.
- **Cursor switch now attempts to launch the default instance after injection**: switching a Cursor account now updates the default-instance binding and tries to start Cursor immediately, while still emitting unified path-missing guidance when the app path is unavailable.
- **Codex quota presentation is now consolidated into one flexible column**: the Accounts table now renders all quota windows in a single area, and Code Review quota visibility can be toggled from preferences.

### Fixed
- **Codex account switching now respects `CODEX_HOME`**: Codex auth file read/write now honors custom `CODEX_HOME` (including quoted env values), and auth write errors now include explicit target paths for troubleshooting.
- **Secondary windows no longer inherit main-window close interception**: non-main windows now close directly instead of being incorrectly blocked by the tray/minimize confirmation flow.
- **Windsurf safe-storage key lookup is now provider-specific**: macOS and Linux credential handling no longer falls back to generic VS Code safe-storage entries, reducing wrong-key reads during injection.

---
## [0.11.3] - 2026-03-10

### Fixed
- **Gemini OAuth app identity now matches official Gemini CLI**: Gemini authorization now uses the official Gemini CLI OAuth client credentials, so the consent page aligns with `Gemini Code Assist and Gemini CLI` instead of legacy app identity.
- **Gemini web OAuth callback flow now aligns with official behavior**: the browser auth URL uses the official parameter set (without extra `prompt=consent`), and callback handling now redirects to the official Gemini success/failure pages.

---
## [0.11.2] - 2026-03-08

### Fixed
- **Antigravity default-instance custom launch args now take effect**: launching the default Antigravity instance now parses and passes saved `extra_args` to the actual process start command.
- **Remote debugging launch flags can now be applied from Cockpit settings**: flags such as `--remote-debugging-port=9333` are no longer silently dropped in the default-instance start path.

---
## [0.11.1] - 2026-03-08

### Changed
- **Gemini import now validates token state immediately**: JSON import and local `~/.gemini` import now trigger a post-import token refresh, so account metadata is synchronized right after import.
- **Gemini refresh state is now persisted on every outcome**: refresh failures now write `status=error` plus `status_reason` to the account record, and successful refreshes clear the error status.

### Fixed
- **Gemini refresh failures no longer stay as log-only signals**: failed manual or batch refresh attempts are now persisted to account status fields for consistent UI visibility.

---
## [0.11.0] - 2026-03-08

### Added
- **Gemini platform full integration across backend and frontend**: Added Gemini models/commands/modules/OAuth on Tauri side, plus account pages, services, stores, icons, navigation, and platform metadata wiring on frontend.
- **Gemini account lifecycle support**: Added OAuth login, Access Token/JSON import, local `~/.gemini` import, quota refresh, tag management, account export, and local credential injection for account switching.
- **Gemini multi-instance management**: Added Gemini instance store/commands with default and custom profile directories, account binding/injection, launch command generation, and one-click terminal execution.
- **Gemini settings and runtime integration**: Added `gemini_auto_refresh_minutes`, Gemini quota-alert enable/threshold config, and integrated Gemini into Settings, Quick Settings, auto-refresh scheduler, dashboard, and tray/runtime surfaces.
- **Gemini docs and i18n coverage**: Updated README (EN/ZH) and locale keys for Gemini overview, instance workflows, switching, importing, and flow notices.

### Changed
- **Post-switch UX now supports provider-specific success actions**: `useProviderAccountsPage` now exposes an inject-success callback; Gemini overview uses it to open a launch-command modal immediately after switching.
- **Gemini launch semantics aligned with default-instance behavior**: Default-instance launch command now uses plain `gemini`; custom instances keep `GEMINI_CLI_HOME=... gemini`.
- **Gemini launch modal wording updated for generic use**: Launch dialog title now uses “Launch Instance” instead of a multi-instance-specific label.
- **Gemini instance UI simplified to match actual CLI behavior**: Removed runtime-state/PID/stop expectations in Gemini instance list and aligned default-instance edit behavior with real launch semantics.
- **Shared platform/presentation pipeline expanded for Gemini**: Added Gemini to shared platform typing/navigation/meta and unified Gemini account plan/quota presentation in reusable account view helpers.

---
## [0.10.1] - 2026-03-07

### Added
- **Cursor platform end-to-end integration**: Added full Cursor account and multi-instance support across backend commands/modules, frontend pages/stores/services, side navigation, tabs, dashboard cards, and tray integration.
- **Cursor account management capability set**: Added OAuth (PKCE), Token/JSON import, local `state.vscdb` import, account export, and account injection back to Cursor profile data for switching.
- **Cursor quota and subscription pipeline**: Added official refresh chain (`usage-summary`, `GetUserMeta`, Stripe profile endpoints), including Total/Auto/API/On-Demand metrics and team-limit parsing.
- **Cursor settings and automation wiring**: Added Cursor app path, auto-refresh interval, and quota-alert enable/threshold in both Settings and Quick Settings, and integrated Cursor into global auto-refresh.
- **Cross-platform existing-directory instance mode**: Added `existingDir` initialization mode for Antigravity/Codex/GitHub Copilot/Windsurf/Kiro/Cursor to register existing local directories as instances.
- **Fingerprint preview autofill support**: Added preview-current-profile autofill/writeback for missing fields and frontend field-level auto-generated indicators.

### Changed
- **App framework now includes Cursor globally**: added Cursor routing/page mounting, dashboard current/recommended account card actions, platform typing/navigation expansion, and platform metadata wiring.
- **System tray startup and rendering path was upgraded**: tray now boots with a lightweight skeleton menu first and asynchronously loads full account-driven menus; Cursor tray summaries and platform ordering are included.
- **Startup blocking work was reduced**: settings merge and log cleanup were moved to background threads; i18n startup preloads `en` resources and uses an explicit loading shell before app mount.
- **Settings/Quick Settings/config schema expanded**: added `cursor_auto_refresh_minutes`, `cursor_app_path`, `cursor_quota_alert_enabled`, and `cursor_quota_alert_threshold`, with backward-compatible config normalization.
- **Instance workflow enhancements across providers**: Antigravity/Codex/GitHub Copilot/Windsurf/Kiro/Cursor now validate and support `existingDir` creation mode in backend and frontend forms.
- **Codex account presentation expanded**: added auth metadata parsing (`Signed in with <provider>`, ID details) and a dedicated Code Review quota metric in cards/tables.
- **Plan/tier badge styling unified**: introduced shared `--plan-*` design tokens and switched account/instance pages to common badge color mapping.
- **Localization coverage was expanded for new Cursor flows**: updated locale keys across supported language packs for Cursor pages, OAuth/import flow, `existingDir` instance mode, quick settings, and quota display copy.

### Fixed
- **Provider account dedup correctness improved**: GitHub Copilot and Windsurf now deduplicate by `github_id`; Kiro avoids email-only merges when `user_id` presence conflicts.
- **Kiro account deduplication issue fixed**: Fixed the Kiro account merge path (`b045e1e2`) where different user identities could be incorrectly merged under the same email and cause account overwrite.
- **Fingerprint preview data consistency improved**: reading current profile now autofills missing fingerprint fields, returns generated-field markers, and attempts writeback to storage.
- **Path-missing guidance chain now covers Cursor fully**: `APP_PATH_NOT_FOUND:cursor` is handled by unified set/reset/detect/retry flow.

---
## [0.10.0] - 2026-03-07

### Added
- **Cursor platform end-to-end integration**: Added full Cursor account and multi-instance support across backend commands/modules, frontend pages/stores/services, side navigation, tabs, dashboard cards, and tray integration.
- **Cursor account management capability set**: Added OAuth (PKCE), Token/JSON import, local `state.vscdb` import, account export, and account injection back to Cursor profile data for switching.
- **Cursor quota and subscription pipeline**: Added official refresh chain (`usage-summary`, `GetUserMeta`, Stripe profile endpoints), including Total/Auto/API/On-Demand metrics and team-limit parsing.
- **Cursor settings and automation wiring**: Added Cursor app path, auto-refresh interval, and quota-alert enable/threshold in both Settings and Quick Settings, and integrated Cursor into global auto-refresh.
- **Cross-platform existing-directory instance mode**: Added `existingDir` initialization mode for Antigravity/Codex/GitHub Copilot/Windsurf/Kiro/Cursor to register existing local directories as instances.
- **Fingerprint preview autofill support**: Added preview-current-profile autofill/writeback for missing fields and frontend field-level auto-generated indicators.

### Changed
- **App framework now includes Cursor globally**: added Cursor routing/page mounting, dashboard current/recommended account card actions, platform typing/navigation expansion, and platform metadata wiring.
- **System tray startup and rendering path was upgraded**: tray now boots with a lightweight skeleton menu first and asynchronously loads full account-driven menus; Cursor tray summaries and platform ordering are included.
- **Startup blocking work was reduced**: settings merge and log cleanup were moved to background threads; i18n startup preloads `en` resources and uses an explicit loading shell before app mount.
- **Settings/Quick Settings/config schema expanded**: added `cursor_auto_refresh_minutes`, `cursor_app_path`, `cursor_quota_alert_enabled`, and `cursor_quota_alert_threshold`, with backward-compatible config normalization.
- **Instance workflow enhancements across providers**: Antigravity/Codex/GitHub Copilot/Windsurf/Kiro/Cursor now validate and support `existingDir` creation mode in backend and frontend forms.
- **Codex account presentation expanded**: added auth metadata parsing (`Signed in with <provider>`, ID details) and a dedicated Code Review quota metric in cards/tables.
- **Plan/tier badge styling unified**: introduced shared `--plan-*` design tokens and switched account/instance pages to common badge color mapping.
- **Localization coverage was expanded for new Cursor flows**: updated locale keys across supported language packs for Cursor pages, OAuth/import flow, `existingDir` instance mode, quick settings, and quota display copy.

### Fixed
- **Provider account dedup correctness improved**: GitHub Copilot and Windsurf now deduplicate by `github_id`; Kiro avoids email-only merges when `user_id` presence conflicts.
- **Fingerprint preview data consistency improved**: reading current profile now autofills missing fingerprint fields, returns generated-field markers, and attempts writeback to storage.
- **Path-missing guidance chain now covers Cursor fully**: `APP_PATH_NOT_FOUND:cursor` is handled by unified set/reset/detect/retry flow.

---
## [0.9.17] - 2026-03-06

### Changed
- **Windows Codex path reset detection is now Store-first and drive-aware**: Path reset now scans `C:\Program Files\WindowsApps\OpenAI.Codex_*\app\Codex.exe` and `<Drive>:\WindowsApps\OpenAI.Codex_*\app\Codex.exe` across drives, selects the highest package version, and falls back to Appx `InstallLocation\app\Codex.exe` when direct scan misses.
- **Startup auto app-path probing was removed**: The app no longer runs automatic app-path detection during startup; path detection now runs on explicit reset or launch-missing-path flows.
- **Announcement delivery remains non-intrusive**: New announcements continue to use unread badge indication and no longer force-open the detail modal.

### Fixed
- **Windows Codex default launch now works with configured path**: Added Windows default-instance launch flow for Codex so launch/switch-triggered start can execute when `codex_app_path` is configured.
- **Instance actions no longer stay disabled after stop**: After stopping an instance, row actions are re-enabled in-place without requiring page navigation refresh.
- **Restored macOS Codex multi-instance behavior after 0.9.16 regression**: Reverted the 0.9.16 Codex single-instance restriction impact on macOS, restoring multi-instance launch/control flow to the v0.9.15 behavior baseline.
- **Restored Codex PID recognition on macOS instance rows**: Brought back instance-home-based process matching so running Codex instances can be identified and displayed with PID correctly.

---
## [0.9.16] - 2026-03-05

### Added
- **Windows Codex desktop control management**: Added first-class Windows support for Codex desktop process control, including launch, stop, focus, and restart flow (close first, then reopen).
- **Windows Codex auto path detection for Store installs**: Added Appx-based path detection via `OpenAI.Codex` `InstallLocation\\app\\Codex.exe`, so Cockpit can resolve Codex executable path in Microsoft Store installation scenarios.

### Changed
- **Announcement delivery is now non-intrusive by default**: New announcements no longer force-open the detail modal; unread items are indicated by the red badge only and are opened manually from Announcement Center.
- **Codex account identity display is now compact and single-line**: Codex account cards/tables now show `Signed in with <provider> | Account ID: <id>` in one line, and workspace name is removed from default account identity display to reduce UI noise.
- **Codex code review quota label is fixed to English**: The code review quota metric now always uses `Code Review` as the display label.
- **Windows Codex control model aligned to official single-instance lock**: Codex multi-instance is now explicitly marked unsupported on Windows/macOS in Codex instance management, with clear UI/backend reason text, and operations are constrained to single-instance control semantics.

---
## [0.9.15] - 2026-03-04

### Changed
- **Release publication now waits for full pipeline completion**: The release workflow now creates draft releases first and only marks them as `latest` after matrix builds, merged updater `latest.json`, checksum upload, and Homebrew cask update all succeed. This prevents in-app update prompts from appearing before release artifacts are fully ready.

---
## [0.9.14] - 2026-03-04

### Added
- **Floating sidebar quick-update action**: Added a compact action above the sidebar that follows updater lifecycle states (`Update` / `Downloading %` / `Restart`), so users can continue update flow without reopening settings pages.

### Changed
- **Updater retry and failure handling hardened**: Added retry-with-backoff plus retryable/non-retryable error classification for update check/download, with retry status feedback and sanitized error details in UI/logs.
- **Update check baseline interval changed to 1 hour**: Default update-check interval is now 1 hour, and legacy 6h/24h interval values are migrated to 1 hour automatically when settings are loaded.
- **macOS process probe path for desktop clients was switched to `ps`-first matching**: Antigravity/VS Code/Codex/Kiro/Windsurf process discovery now prioritizes command-line probing and keeps app-root path comparison, reducing protected-directory touches while improving process match stability for instance operations.
- **Antigravity macOS multi-instance startup behavior was tightened**: Non-default instances now launch via `open -n` without `--reuse-window`, and startup includes a short PID resolve polling window (up to 6s) for the target `user-data-dir`.
- **Codex quota refresh now synchronizes plan metadata back to account index**: `plan_type` is now synced from refreshed `id_token` and quota usage response into account summary index, so subscription badges can update without re-import.

### Fixed
- **Reopened update dialog now preserves prepared-update restart state**: If the same version is already downloaded, reopening the update dialog now stays in `Ready to restart` state instead of falling back to `Update now`.
- **Manual dialog restart now reuses unified apply-update pipeline**: `Restart now` in the update dialog now follows the same install/relaunch path as silent updates, preventing state divergence between update entry points.
- **GitHub Copilot instance injection no longer fails on macOS due to wrong Safe Storage key priority**: VS Code/Copilot injection now prefers Code-family Keychain entries before Antigravity entries, preventing `AES-CBC decryption failed: Unpad Error` when decrypting existing `github.auth`.

---
## [0.9.13] - 2026-03-03

### Added
- **Pending update notes local cache for post-restart changelog**: Added persisted `pending_update_notes.json` storage so downloaded update notes can be shown after restart without requiring online changelog fetch.

### Changed
- **Update check source is now fully unified to Tauri Updater metadata**: Removed backend GitHub Releases API polling for version detection; update availability now comes from updater endpoint metadata (`latest.json`) only.
- **Manual/silent update note rendering now reads updater release body**: Update dialog and silent-update pre-cache now parse bilingual sections directly from updater `notes`, while keeping browser-download fallback only for updater failures.

---
## [0.9.12] - 2026-03-03

### Added
- **Background auto update mode (zero-intervention)**: Added a `Settings > General > Background Auto Update` option. When enabled, the app checks updates normally, downloads new packages silently in the background, and prompts restart when the update is ready.
- **Post-update changelog popup on version jump**: Added startup version-jump detection based on locally recorded `last_run_version`. After an upgrade, the app now shows a “What’s New” dialog for the current version.
- **Silent update ready toast with restart action**: Added a bottom-right update toast after background download, with `Later` and `Restart` actions.

### Changed
- **Desktop updater pipeline migrated to Tauri Updater**: Integrated updater/process plugins and enabled updater artifacts + release endpoint config, so in-app update flow uses signed updater metadata.
- **Manual update dialog now supports in-app download/install progress**: The update modal now performs in-app update with progress/status/error display and falls back to opening the GitHub release page when updater flow fails.
- **Update settings persistence behavior was hardened**: Auto-update preference loading/saving now avoids first-render overwrite and only writes when user change or explicit state change is confirmed.
- **Update-related i18n coverage expanded across locale packs**: Added update toggle/progress/restart/version-jump translation keys in all supported locales.

---
## [0.9.11] - 2026-03-03

### Fixed
- **Fixed Windows switch/start crashes on non-ASCII install paths**: Windows extended-path normalization now uses Unicode-safe prefix handling, preventing `byte index is not a char boundary` panics on non-ASCII paths.

### Changed
- **Verification default model selection now prioritizes Flash**: In “Run check now”, the default model now selects the first option whose display name contains `flash` (case-insensitive), and falls back to the first available model when no match exists.

---
## [0.9.10] - 2026-03-02

### Changed
- **Official-aligned wakeup execution stability**: Extended official LS startup wait to 60s, aligned client-gateway trajectory polling window to 60s, and switched `app_data_dir` to an official-style IDE-level directory (`antigravity`, overridable by `AG_WAKEUP_OFFICIAL_LS_APP_DATA_DIR`).
- **Wakeup gateway error-handling flow now mirrors long-running cascade behavior**: When trajectory status remains `RUNNING`, intermediate `errorMessage` steps are treated as transient and polling continues before final fail/success decision.
- **Antigravity plan badge rendering unified across account surfaces**: Centralized tier badge mapping via `getAntigravityTierBadge` and reused it in Accounts and verification detail surfaces.
- **Instance account selector ordering now follows Accounts sorting across all platforms**: Account dropdown ordering in multi-instance views now reuses each platform’s Accounts sort logic (Antigravity / Codex / GitHub Copilot / Windsurf / Kiro), avoiding cross-page ordering drift.
- **Accounts sort preferences are now persisted for all platforms**: Sort field and sort direction in all account pages now persist to local storage and are restored after restart.
- **Instances list sort preferences are now persisted per platform**: Instance list sort field (`createdAt` / `lastLaunchedAt`) and direction now persist by app type, so restart no longer resets instance list sorting.

### Fixed
- **Temporary upstream failures now self-retry once in wakeup path**: `temporary`/HTTP 5xx style payloads from `AG_WAKEUP_ERROR_JSON` now trigger a delayed one-time retry before returning failure.
- **Wakeup verification detail now shows backend user-facing error message**: Detail list now renders `lastMessage` (with truncation), so messages like `Agent execution terminated due to error.` are visible.
- **Wakeup tasks now respect privacy masking for account emails**: Masking is now applied in task cards, test selectors, history rows, and copied debug text.

---
## [0.9.9] - 2026-03-02

### Added
- **Built-in User Manual page**: Added a new `manual` page with scenario-based sections (quick start, dashboard, provider accounts, multi-instance, fingerprints, wakeup/verification, settings, and import/export + troubleshooting), keyword search, and expand/collapse controls.
- **One-click manual entry points across key pages**: Added manual shortcuts to dashboard/header areas and account empty states (Antigravity, Codex, GitHub Copilot, Windsurf, Kiro) to reduce first-run friction.

### Changed
- **Manual page now supports direct action shortcuts**: Each section can jump directly to related pages (Dashboard, Antigravity, Codex, GitHub Copilot, Windsurf, Kiro, Multi-Instance, Fingerprints, Wakeup Tasks, Verification, Settings), and can open Platform Layout from the guide.
- **Manual localization coverage expanded across locale packs**: Added `manual.*` keys and `nav.manual` labels across all supported locale files to keep guide/navigation copy consistent in multi-language environments.

### Fixed
- **Fixed permission-prompt attribution to Cockpit when launching third-party apps on macOS**: When launching Antigravity/Codex/GitHub Copilot/Windsurf/Kiro from Cockpit, protected-directory permission prompts are now significantly less likely to be attributed to Cockpit.

---
## [0.9.8] - 2026-03-01

### Changed
- **Refactored AccountsPages across 4 platforms (Codex/GitHub Copilot/Windsurf/Kiro)**: Introduced `useProviderAccountsPage` plus shared data extraction utilities to consolidate shared state/actions and reduce duplicated page logic.
- **Unified export UX across account pages**: Added `ExportJsonModal` + `useExportJsonModal`, aligned multi-account/single-account export flows, and added download-directory open capability permissions for the export modal flow.
- **Standardized OAuth copy and tab naming across locales**: Updated add-account OAuth labels/description copy to consistently use “OAuth Authorization”.
- **OAuth post-login now performs best-effort refresh**: Added post-login refresh passes for Antigravity quota and GitHub Copilot/Windsurf/Kiro token snapshots to reduce stale data right after authorization.
- **Path-missing guidance now carries retry context**: App-path guidance payload now supports `switchAccount` / `default` / `instance` retry intents so path save can continue the original user action.
- **Wakeup behavior switched to strict no-fallback mode**: Wakeup execution now requires explicit `project_id`; model fetch no longer falls back to hardcoded lists; scheduler no longer uses `fallback_times` outside the time window.
- **Instance window operation semantics tightened**: “Open instance window” now reports focus failures directly instead of auto-starting new processes.
- **Account identity matching is stricter**: Removed email-only merge fallback in Antigravity/Codex account matching paths; Codex-to-OpenCode auth payload now uses persisted `account_id` only.
- **Token parsing/refresh rules tightened for Windsurf/Kiro**: Windsurf token import accepts only API key or Firebase JWT formats; Kiro refresh now fails explicitly when refresh cannot be performed (no snapshot fallback).
- **Command trace pipeline added and made opt-in**: Added trace points for command EXEC/RESULT/SPAWN paths and kept it disabled by default unless `COCKPIT_COMMAND_TRACE=1`.
- **Quick settings quota-alert controls were componentized**: Extracted duplicated quota-alert UI logic into a shared rendering path in quick settings.

### Fixed
- **Launch-path validation now runs before switch/start execution**: When path is missing/invalid, backend returns `APP_PATH_NOT_FOUND:*` before stop/inject/restart actions.
- **Windows focus flow no longer hits `$PID` overwrite errors**: Focus scripts switched to a dedicated PID variable and retry loop for non-zero `MainWindowHandle` before calling focus APIs.
- **Windows executable-path matching reliability improved**: Added normalization for extended path prefixes (`\\?\`, `\\?\UNC\`), environment expansion, command-line exe extraction fallback, and sysinfo fallback diagnostics for path probe misses.
- **Path-missing guidance modal now matches settings visual style**: Reused quick-settings/settings shared styles for consistent title/path section/icon/typography/layout behavior.
- **Fixed Rust warnings in backend integration paths**: Cleaned warning points in token model and wakeup gateway reserved code paths so refactor branch warnings stay controlled.

---
## [0.9.7] - 2026-02-28

### Fixed
- **macOS repeated privacy permission prompts suppressed**: Replaced broad `sysinfo` process refresh (which fetched `cwd`/`environ`/`root` for all processes) with targeted `ProcessRefreshKind` requests that only retrieve `exe` and `cmd`. This prevents sysinfo from touching protected directories on other processes and eliminates the repeated Music/Photos/Documents permission dialogs on macOS.

### Changed
- **Kiro/Windsurf quota cycle reset time now shows relative + absolute format**: `formatKiroResetTime` rewritten to output `Xd Xh (MM/DD HH:mm)` style, consistent with other platform reset time displays. Sub-day granularity now shows hours/minutes instead of rounding to days.
- **Kiro/Windsurf cycle remaining time shows hours when under 24 hours**: Quota cycle remaining text now switches to `Resets in Xh` when less than one day remains, instead of showing `0 days`.
- **Kiro dashboard card quota display simplified**: Removed redundant used/total and left lines from the Kiro mini-card in Dashboard; now shows `resetText` or `cycleText` directly, consistent with Windsurf card layout.

---
## [0.9.6] - 2026-02-28


### Changed
- **Unified account presentation pipeline across five platforms and multiple entry pages**: Added a shared presentation layer for display name, plan label (raw value), quota metrics, reset text, and usage summaries, and reused it in Dashboard, Accounts, and Instances pages (Antigravity / Codex / GitHub Copilot / Windsurf / Kiro) to avoid multi-place divergence.
- **Token import UX now provides concrete input examples**: Updated token/JSON placeholder copy across locales and added token-format helper styling to improve readability in add/import modals.

### Fixed
- **Antigravity tray quota lines now follow group settings**: Tray submenu now aggregates by configured display groups (including model alias compatibility), so tray output matches grouped quota cards instead of raw per-model lines.
- **Tray refresh now reacts immediately to group-setting updates**: Saving/changing/deleting/reordering groups triggers tray menu refresh without requiring restart/manual cache actions.
- **Re-added accounts can reuse previous fingerprint binding after deletion**: Added deleted-account fingerprint binding persistence and lookup, so delete/re-add flows preserve original fingerprint association when available.
- **Antigravity plan badge display is now unified to normalized tiers**: Instance/account surfaces now consistently show `PRO/ULTRA/FREE/UNKNOWN` instead of mixed raw subscription-tier strings.
- **Antigravity token example helper copy now fully uses i18n keys**: Removed hardcoded Chinese labels in the token example panel so locale switching stays consistent.

## [0.9.5] - 2026-02-28

### Fixed
- **Windows wakeup no longer pops black terminal windows**: Added hidden-process flags for official Language Server startup and Windows CLI probes (`netstat`, `where.exe`) used by wakeup-related flows.
- **Local wakeup gateway intermittent transport failures now self-recover once**: Added local health-check preflight, transport error classification, and one-time gateway cache rebuild retry for recoverable local connection/TLS/timeout failures.
- **Local gateway requests now bypass system proxy and use a canonical loopback address**: Gateway/official-LS local clients now enforce `no_proxy`, and loopback base URLs are normalized to `127.0.0.1` to reduce proxy/interception-related failures.

### Changed
- **Verification copy and action labels switched from “Verify” to “Detect” across all locales**: Added/used `wakeup.verification.actions.runCheckNow`, updated run-hint wording, and aligned the verification-page primary CTA/title.
- **GitHub Copilot instances quota row now includes Premium requests**: Instance account quota summary now shows Inline, Chat, and Premium usage percentages together.

---
## [0.9.4] - 2026-02-27

### Fixed
- **Linux `.deb` blank/white window rendering on some environments**: Disabled transparent window by default (`transparent: false`) and added Linux WebKitGTK fallback (`WEBKIT_DISABLE_DMABUF_RENDERER=1` when unset) to improve render stability.
- **Windows account-switch flow could hang while probing Antigravity processes**: Added a 5-second timeout for PowerShell process probing and automatic fallback to `sysinfo` scanning to avoid blocking the switch path.
- **Switch-success but launch-failure now becomes user-visible**: If account data is switched but launching Antigravity fails, backend now returns an explicit error message so frontend can show a visible failure notice.
- **Official LS resolution now follows configured Antigravity app path on all desktop OSes**: Wakeup/verification now derive LS from `antigravity_app_path` on Windows/macOS/Linux (with platform-specific extension/bin path and filename priority), and return unified `APP_PATH_NOT_FOUND:antigravity` when missing so existing path-setup guidance is triggered before execution.

---
## [0.9.3] - 2026-02-27

### Fixed
- **AppImage blank-page rendering on Linux (including Arch) caused by absolute asset paths**: Vite build output now uses relative asset paths (`base: "./"`), so packaged AppImage can resolve frontend JS/CSS correctly.

### Changed
- **Release-process documentation aligned to current completion rule**: Updated `docs/release-process.md` to treat `remote branch + remote tag` as release completion, while GitHub Actions/asset publishing remains a post-release async step.

---
## [0.9.2] - 2026-02-27

### Changed
- **Windows wakeup/verification now prechecks runtime readiness before execution**: Added a frontend + backend preflight gate so wakeup test and batch verification validate official LS readiness first, instead of failing after request dispatch.
- **Official LS path resolution now derives from configured Antigravity app path on Windows**: Runtime now resolves LS from the configured `antigravity_app_path` (`resources/app/extensions/antigravity/bin`), with deterministic filename priority and fallback matching in the same bin directory.

### Fixed
- **Path-missing guidance now triggers before wakeup starts**: When Antigravity app path or LS binary is unavailable on Windows, the existing `app-path-missing` flow is triggered immediately, preventing late 500 errors from gateway startup.

---
## [0.9.1] - 2026-02-27

### Added
- **Announcement system (desktop)**: Added a full announcement pipeline with Tauri commands, frontend store/service/types, and announcement center UI (list/detail modal, unread badge, mark-read, refresh, popup, image preview, and action handling for tab/url/command).
- **Announcement source controls for dev testing**: Added local override support (`~/.antigravity_cockpit/announcements.local.json`) and debug workspace source (`announcements.json`) for `npm run tauri dev` testing, with persisted read-state/cache files.
- **Repository announcement seed file**: Added a repository-level `announcements.json` with a welcome announcement and feedback action for quick local debugging and remote source alignment.

### Changed
- **Remote-first announcement strategy for normal users**: Non-dev/runtime builds now skip local override files and use remote announcements (with cache/fallback) by default.
- **Dashboard header action area**: Replaced the dashboard date display with an inline `Announcement` action button; announcement entry is now shown in dashboard context instead of global full-page placement.
- **v0.9.0 announcement content is now fully localized**: Added/filled title, summary, body, and action copy for all 17 supported languages in `announcements.json`, so users see localized announcement content per language environment.
- **GitHub Copilot usage rendering alignment (dashboard + tray)**: Switched usage parsing to structured snapshot semantics (`completions` / `chat` / `premium_interactions`), added `Included` handling, and added a `Premium` metric line/dimension in both dashboard cards and tray summaries.
- **Locale and copy coverage for announcement/tray semantics**: Added `announcement.*` keys across all locale files and extended tray copy mapping with `Included` and GitHub Copilot metric labels (`Inline` / `Chat` / `Premium`).

---
## [0.9.0] - 2026-02-27

### Added
- **Dedicated Antigravity account verification workspace**: added model-based batch account verification with live progress, persisted verification history, per-batch detail view, and status filters (`All` / `Success` / `Verification required` / `Failed`).
- **Official-aligned wakeup/verification transport**: added a `local gateway + official Language Server protocol` flow using `StartCascade` / `SendUserCascadeMessage` / `GetCascadeTrajectory` / `DeleteCascadeTrajectory` for wakeup conversations and account verification runs.
- **403 verification quick actions**: verification-required results now expose validation URL and actions (`Verify now`, copy validation URL, copy debug info) for self-service verification.

### Changed
- **Unified model-list rule across wakeup surfaces**: wakeup task model picker, verification picker, and quota-related model display now all derive from official `agentModelSorts[].groups[].modelIds`; when unavailable, fallback is limited to the fixed 6 recommended models.
- **Antigravity model grouping reduced to 3 groups**: default display groups are now `Claude / Gemini Pro / Gemini Flash`; `Gemini Image` group and legacy mapping are removed to avoid duplicate group rendering.
- **Verification-page UX and privacy alignment**: added batch selection/deletion flow, closable notices, and privacy-toggle-linked email masking consistent with the Accounts page.
- **GitHub Copilot (VS Code semantics) display alignment**: `individual` plans are now normalized to `PRO`; usage is derived from `quota_snapshots.completions/chat/premium_interactions`; cards and tables now include a `Premium requests` dimension with `Included` display support.
- **Wakeup custom-time interaction refinement**: custom time keeps a `time picker + quick input` interaction; empty state no longer shows a default time value; custom time input is now applied to next-run preview and task save even if `Add` is not clicked.

---
## [0.8.13] - 2026-02-24

### Added
- **Independent Dock icon visibility setting (macOS only)**: Added a `Hide Dock icon` option in Settings > General so Dock icon visibility can be controlled separately from close/minimize behavior.
- **Localization coverage for macOS window-behavior options**: Added translation keys for `minimizeBehavior` and `hideDockIcon` settings across supported locales.

### Changed
- **macOS window-behavior config model split**: Added persistent `minimize_behavior` and `hide_dock_icon` fields in local config and wired them through Tauri system commands and WebSocket config updates; startup now applies the Dock activation policy from saved config.
- **Tag edit modal visual polish (especially dark theme)**: Improved dark-theme background, borders, chip/remove-button styling, and input/placeholder/disabled states.
- **OAuth auth URL parameter cleanup**: Removed `include_granted_scopes=true` from generated OAuth authorization URLs.

### Fixed
- **macOS Dock visibility now updates immediately after saving settings**: Changing the Dock icon visibility option now reapplies the macOS activation policy without requiring an app restart.
- **Language-switch config saves preserve new macOS window fields**: WebSocket language updates now keep `minimize_behavior` and `hide_dock_icon` when writing config, avoiding accidental resets.

---
## [0.8.12] - 2026-02-22

### Added
- **One-command GitHub Release + Homebrew Cask publisher**: Added `scripts/release/publish_github_release_and_cask.cjs` and `npm run release:github-and-cask` to build a `universal.dmg`, upload assets to GitHub Release, and update `Casks/cockpit-tools.rb` (with `--skip-build` / `--skip-gh` / `--skip-cask` / `--dry-run` support).

### Changed
- **Startup app-path detection strategy**: On startup, the app now loads local config first, probes only platforms without configured paths, and staggers detection calls with a small delay to reduce bursts of system path-detection commands.
- **Release-process docs expanded for Homebrew flow**: Updated `docs/release-process.md` with recommended `universal` build flow, checksum generation examples, GitHub CLI/Rust target prerequisites, and cask update ordering notes.
- **Release workflow restores automatic Homebrew Cask updates**: `release.yml` now restores the `update-homebrew-cask` job to compute `sha256` from the published `*_universal.dmg`, update `Casks/cockpit-tools.rb`, and open a cask PR after release assets are available.
- **Auto-merge is limited to generated cask PRs only**: The release workflow now enables auto-merge only for Homebrew cask PRs created on `automation/update-cask-v*` branches (squash + delete branch), without affecting normal PRs.

### Fixed
- **Windows black console flashes during startup**: Fixed unhidden `cmd /c reg query` calls in the VS Code registry fallback path detection flow. Background commands now run hidden, reducing startup black-window flashes for some Windows users.
- **Brand names and plan/tier labels incorrectly localized**: Restored original brand/product names and raw plan labels in non-English locales, including `Cockpit Tools`, `Antigravity`, `Codex`, `GitHub Copilot`, `Windsurf`, plus `accounts.tier.*`, `codex.plan.*`, and `kiro.plan.*`.
- **Locale-check false positives for brand names**: Added brand-name allowlist entries to the locale validation script so English brand strings are not flagged as missing localization.

---
## [0.8.11] - 2026-02-22

### Changed
- **Antigravity quota backend fetch flow aligned with Antigravity.app**: Unified Cloud Code base URL selection for `loadCodeAssist` / `onboardUser` / `fetchAvailableModels` (Antigravity-style routing), passed `cloudaicompanionProject` through backend requests, and switched `onboardUser` to operation polling (`POST` + `GET`, 500ms poll interval). Local quota API cache is still retained, while the pre-cache backend flow is aligned.

---
## [0.8.10] - 2026-02-22

### Added
- **Windsurf email/password account import**: Added an `Email & Password` tab in Windsurf Add Account modal and wired Firebase sign-in flow to create local Windsurf accounts.

### Changed
- **Windsurf credits semantics aligned with monthly quota**: `availablePromptCredits` / `availableFlexCredits` are now treated as monthly total quota, and remaining credits are computed as `total - used`.
- **Password handling in sign-in flow**: Email is still normalized with trim, while password now keeps the original input (no trim) to avoid altering valid credentials.

### Fixed
- **Windsurf password-login i18n coverage**: Added missing `windsurf.addModal.password` and `windsurf.password.*` translation keys across locale files to prevent fallback language leakage.
- **Password-login log privacy**: Removed plain email output from Windsurf password-login logs to reduce PII exposure.

---
## [0.8.9] - 2026-02-21

### Added
- **Account card tags are now visible across all five platforms**: Account tags now render directly on grid cards in Antigravity, Codex, GitHub Copilot, Windsurf, and Kiro for faster visual identification.

### Changed
- **Card tag display is unified**: Tag chips now follow a consistent compact rule across platforms (show up to 2 tags with `+N` overflow).

### Fixed
- **Release checksum upload workflow no longer depends on local git checkout**: Added explicit `GH_REPO` context for `gh release` calls in the checksum upload job to avoid `fatal: not a git repository` failures.

---
## [0.8.8] - 2026-02-21

### Changed
- **Codex quota windows now follow window presence**: Codex quota rendering is now driven by `primary_window` / `secondary_window` presence instead of always forcing two fixed lines.
- **Codex window labels now use Codex-style rules**: Window labels now use unified dynamic formatting (`5h`, `Weekly`, `Xd`, `Xh`, `Xm`) based on actual window minutes.
- **Multi-instance Codex account selector now shows plan badge**: Bound-account dropdown/list in Codex instances now shows subscription badges (`FREE/PLUS/PRO/TEAM/ENTERPRISE`) alongside account emails to reduce free-plan ambiguity.
- **Manual update check now always shows feedback**: Clicking `Check Updates` now shows loading state and explicit result feedback (`up to date` / `check failed`) instead of silent no-op when no new version is found.
- **Release workflow now auto-publishes checksums**: GitHub Release pipeline now automatically generates and uploads `SHA256SUMS.txt` from release assets, removing manual checksum upload.

### Refactored
- **Shared Codex quota-window helper introduced**: Codex account page, dashboard cards, and Codex instances now reuse the same window-label/window-visibility helper to keep display logic consistent.

---
## [0.8.7] - 2026-02-21

### Changed
- **Unknown-tier rendering and filtering added**: Accounts with missing subscription tier now resolve to `UNKNOWN` (instead of falling back to `FREE`) in cards/tables, and the account filter dropdown now supports `UNKNOWN` as a dedicated option.
- **Unknown badge now uses warning styling**: `UNKNOWN` tier badges are highlighted in red to visually distinguish tier-identification anomalies from normal `FREE` accounts.
- **Quota modal badge consistency**: Quota details modal now always shows a tier badge, including `UNKNOWN` when subscription tier is unavailable.

### Fixed
- **No stale tier carry-over after refresh**: Removed backend behavior that preserved previous `subscription_tier` when the new quota payload had no tier, preventing old `PRO/ULTRA` labels from persisting incorrectly.
- **Tier-identification diagnostics improved**: Subscription identification logs now emit explicit `UNKNOWN` failure reasons (including status/body snippets and loadCodeAssist context) to distinguish API errors from successful responses without tier data.

---
## [0.8.6] - 2026-02-21

### Changed
- **Model group auto-classification now ignores version suffixes**: Added prefix/pattern matching for model families so Claude and Gemini variants are grouped by family (Pro/Flash/Image) even when exact IDs are not pre-listed.
- **"Other Models" cleanup for Claude/Gemini variants**: Claude Sonnet/Opus variants and Gemini x Pro/Flash/Pro Image variants are now routed into their target default groups instead of falling into `Other Models`.
- **Default Gemini group labels renamed**: Group display names were updated from `G3-Pro`, `G3-Flash`, `G3-Image` to `Gemini Pro`, `Gemini Flash`, `Gemini Image` for version-agnostic naming.

### Fixed
- **Legacy group-name compatibility**: Existing saved group settings with legacy `G3-*` names are automatically migrated to the new Gemini labels on load.

---
## [0.8.5] - 2026-02-19

### Added
- **Kiro account ban detection**: Automatic detection of suspended/banned Kiro accounts. When the quota refresh API returns a ban signal (e.g. 403 FORBIDDEN), the account is automatically marked as `banned` with the reason stored.

### Changed
- **Banned account UI**: Account cards and table rows now show a 🔒 `forbidden` status badge and a greyed-out card style to visually distinguish banned accounts.
- **Banned account action restrictions**: The switch button is disabled for banned accounts; the dashboard recommendation algorithm and quota alert suggestions automatically exclude banned accounts.
- **Bulk refresh skips banned accounts**: Refresh-all now skips accounts already marked as banned, reducing unnecessary API calls, and logs the skipped count.
- **Quota alert excludes banned current account**: If the currently active account is banned, quota alert checks are skipped.

### Fixed
- **Error vs. ban state separation**: Refresh failures (`error`) and account bans (`banned`) are now recorded separately, preventing all refresh errors from being misclassified as generic errors.

---
## [0.8.4] - 2026-02-19

### Changed
- **Kiro JSON import now supports raw account snapshots**: The import pipeline now accepts Kiro-style raw JSON objects (and arrays) with fields like `accessToken`, `refreshToken`, `expiresAt`, `provider`, `profileArn`, and `usageData`, then maps them into normalized local accounts.
- **Kiro import parser is unified with OAuth snapshot mapping**: JSON import now reuses the same snapshot-to-payload extraction path as OAuth/local import, improving consistency of email/user/provider/plan/quota field parsing.

### Fixed
- **Slash datetime parsing for imported expiry**: Kiro token expiry values in `YYYY/MM/DD HH:mm:ss` format (e.g. `2026/02/19 02:01:47`) are now parsed correctly during import.
- **Bonus expiry fallback coverage**: `freeTrialExpiry` is now recognized as a fallback source when deriving Kiro add-on expiry days.

---
## [0.8.3] - 2026-02-18

### Changed
- **Tray platform matrix expanded with Kiro**: Added Kiro to tray platform ordering/display and account-count aggregation, and introduced Kiro account summary rendering in tray menus (plan + prompt/add-on remaining with reset time).
- **Legacy tray layout compatibility for Kiro rollout**: When loading old four-platform tray configs, Kiro is auto-appended only for legacy default layouts while preserving user-customized visibility/order.
- **Raw plan/tier labels enforced in account pages**: Antigravity/Codex/GitHub Copilot/Windsurf account cards, tables, and filter options now show original plan/tier values directly (no localized remapping).

### Fixed
- **Auto-switch threshold boundary**: Account auto-switch trigger now fires when remaining percentage is less than or equal to threshold (`<=`) to avoid missing boundary cases at exact threshold.

---
## [0.8.2] - 2026-02-18

### Changed
- **OAuth callback server hardening**: Rewrote the local OAuth callback server to loop over incoming requests, silently ignoring non-callback requests (e.g. favicon), and only processing the actual `/oauth-callback` path. Added CORS preflight (OPTIONS) support and an explicit 404 response for unmatched routes.
- **OAuth CSRF protection**: OAuth authorization URL now includes a `state` parameter generated per flow; the callback server validates the returned state to prevent cross-site request forgery.
- **OAuth flow timeout & cleanup**: Added a configurable timeout for the OAuth callback wait; on timeout or failure the flow state is automatically cleaned up, and a user-facing retry message is returned.
- **OAuth redirect host normalization**: Changed OAuth redirect URI from `127.0.0.1` to `localhost` for broader browser/OS redirection compatibility.
- **Account identity matching overhaul**: Replaced the previous email-only account matching with a strict multi-factor identity matcher (`session_id` → `refresh_token` → `email + project_id`), plus a legacy single-email fallback for backward compatibility during upsert.
- **Google user ID persistence**: OAuth `UserInfo` now parses and stores the Google `id` field, writing it into account data on login completion.

---
## [0.8.1] - 2026-02-17

### Changed
- **Plan/Tier labels now use raw values**: Account-card and table badges across Antigravity/Codex/GitHub Copilot/Windsurf/Kiro now display original backend/local plan values directly, while keeping existing style mapping.
- **Overview tabs use fixed default labels**: Platform overview tabs (`Account Overview` / `Multi-Instance`) now render default text directly to avoid cross-locale mismatch from platform-specific translation overrides.
- **Platform names are fixed to source labels**: Shared platform label rendering now always shows original platform names (`Antigravity`, `Codex`, `GitHub Copilot`, `Windsurf`, `Kiro`).
- **Codex switch behavior is configurable**: Added `codex_launch_on_switch` to backend/user config and wired it into Settings and Quick Settings so switching Codex can optionally skip auto launch/restart.

### Fixed
- **Dashboard privacy consistency**: Dashboard account emails are now masked by the same privacy toggle used in account/instance pages, with focus/visibility/storage sync to keep masking state consistent.
- **OpenCode switch-token sync reliability**: Fixed a regression where GPT account switching did not effectively replace OpenCode login credentials in runtime scenarios, causing the app session to stay on the previous account. (#51)
- **Dashboard card layout balance**: Fixed the Antigravity account card width behavior to avoid obvious right-side whitespace in dashboard layouts and improve visual balance. (#49)

---
## [0.8.0] - 2026-02-17

### Added
- **Fifth platform is live**: Kiro officially joins the supported platform lineup with unified management alongside Antigravity, Codex, GitHub Copilot, and Windsurf.
- **Core Kiro flows are now available**: OAuth/Token/JSON/local import, account switching, quota refresh, multi-instance lifecycle, and app path configuration are all included.
- **Platform-layer refactor**: Instance services, account stores, and overview tabs were unified into reusable platform abstractions to reduce future integration cost.
- **Key fixes in this release**: Hardened Kiro import ID validation against path traversal and filled missing locale keys to reduce mixed-language fallback in non-default locales.

---
## [0.7.3] - 2026-02-15

### Added
- **Tray platform layout persistence**: Added backend tray layout config storage (`tray_layout.json`) and command `save_tray_platform_layout` to save tray visibility, order, and sort mode.
- **Tray visibility control in layout modal**: Added `Show in tray` toggle in platform layout management and synchronized the related locale key across supported languages.
- **Expanded tray platform coverage**: Added GitHub Copilot and Windsurf tray submenus with account/quota summary and direct navigation targets.

### Changed
- **Tray menu architecture**: Refactored tray menu generation to dynamic multi-platform rendering, supporting auto/manual ordering and overflow grouping (`More platforms`).
- **Tray refresh trigger points**: GitHub Copilot/Windsurf refresh, OAuth completion, token import, and account switch flows now refresh tray content immediately; language changes also trigger tray rebuild.
- **Frontend tray event handling**: `tray:refresh_quota` now refreshes Antigravity, Codex, GitHub Copilot, and Windsurf in one flow; tray navigation now recognizes `github-copilot` and `windsurf`.
- **Platform layout sync strategy**: Added debounced frontend-to-backend tray layout sync on reorder/visibility changes and on initial app load.

### Fixed
- **Tray visibility filtering correctness**: Fixed tray platform filtering so disabled platforms remain hidden and the empty-state item appears when no tray platform is selected.
- **Log privacy hardening**: Logger now masks email addresses in log messages to reduce exposure of sensitive identifiers.

---
## [0.7.2] - 2026-02-14

### Added
- **Group management "Other Models" bucket**: Added an auto-collected `Other Models` group that lists non-default models discovered from account quotas.
- **Auth-mode model blacklist filtering**: Added blacklist filtering by model ID/display name in group management to exclude blocked Gemini 2.5/chat variants.
- **Claude Opus 4.6 mapping coverage**: Added `claude-opus-4-6-thinking` and `MODEL_PLACEHOLDER_M26` to default/recommended model mappings and wakeup recommendations.

### Changed
- **Group settings migration behavior**: Loading group settings now incrementally backfills missing default mappings/names/order while preserving user custom configuration.
- **Group modal data source**: Accounts page now passes live model IDs/display names into group management, improving model label display and "other model" classification accuracy.
- **Model display order alignment**: Account quota display order was aligned to include Claude Opus 4.6 under the Claude group.

### Fixed
- **Locale completeness for group settings**: Synchronized `group_settings.other_group` across all supported locales to avoid missing-key fallbacks.

---
## [0.7.1] - 2026-02-14

### Added
- **Cross-platform quota alert workflow**: Added quota alert calculations and event dispatch for Antigravity, Codex, GitHub Copilot, and Windsurf, with per-platform model/credit metric detection.
- **Quota alert settings surface**: Added quota alert enable/threshold controls in both Settings and Quick Settings, with synchronized i18n keys.
- **Global modal infrastructure**: Added reusable global modal store/hook/component (`useGlobalModal` + Zustand + `GlobalModal`) for cross-module prompts and alert actions.
- **Notification capability integration**: Integrated Tauri notification capability in app runtime/capabilities with macOS click-to-focus handling.

### Changed
- **Quota refresh behavior**: Refresh-all and refresh-current flows for Codex/GitHub Copilot/Windsurf now trigger quota-alert checks after successful quota/token refresh.
- **Alert payload model**: `quota:alert` payload now carries `platform`, and the frontend modal quick-switch action now routes to the correct platform switch flow/page.
- **Settings input interaction**: Refresh interval and threshold controls now use preset + inline numeric input mode with Enter/blur apply behavior.
- **Config model propagation**: `quota_alert_enabled` and `quota_alert_threshold` are now persisted through command save/get and websocket language-save paths.
- **Log retention policy**: Logger initialization now cleans up expired `app.log*` files older than 3 days.

### Fixed
- **Quota alert listener lifecycle**: Prevented duplicate `quota:alert` subscriptions caused by async unlisten timing in React effect cleanup.
- **Threshold consistency at 0%**: Runtime threshold normalization now honors `0%`, matching frontend options and user expectations.

---
## [0.7.0] - 2026-02-12

### Added
- **Full Windsurf platform integration**: Added Windsurf account system end-to-end, including OAuth/Token/Local import, account persistence, quota sync, switch/inject/start flow, and multi-instance commands.
- **Windsurf frontend modules**: Added Windsurf account page, instance page, service/store/type layers, and dedicated icon/navigation assets.
- **Dashboard support for Windsurf**: Added Windsurf statistics and overview cards with quick refresh/switch actions, aligned with existing platform cards.
- **Platform layout capability**: Added layout management modal and platform layout store for platform ordering/visibility management in navigation.

### Changed
- **Navigation structure expansion**: Side navigation and routing were extended to include Windsurf and platform-layout entry points.
- **Settings model extension**: General settings now include Windsurf auto-refresh and app-path controls, plus corresponding quick-settings behavior.
- **Windows path detection pipeline**: App detection was upgraded with stronger multi-source probing, PowerShell `-File` fallback, and VS Code registry probing.

### Fixed
- **Path detection reliability on Windows**: Improved handling for empty/error-prone command output and reduced false-miss cases during VS Code/Windsurf path discovery.
- **Quota refresh fallback behavior**: Failed refresh now preserves the last valid quota snapshot to avoid clearing displayed quota to zeros.
- **Switch/injection robustness**: Improved handling and diagnostics around account binding and startup path mismatch cases.

---
## [0.6.10] - 2026-02-10

### Added
- **Privacy mode for screenshots**: Added Eye/EyeOff toggle and masking for email-like identifiers in Antigravity/Codex/GitHub Copilot account overviews and instance pages.
- **GitHub Copilot one-click switching pipeline**: Added default-profile VS Code switching path with token injection and restart integration.
- **Cross-instance window focus/open support**: Added and localized `openWindow` action and improved focus behavior by PID for Antigravity/Codex/VS Code instances.
- **Quota/switch diagnostics**: Added richer runtime logs and metadata outputs for refresh/switch troubleshooting.
- **Codex multi-team identity support**: Added account matching based on `account_id`/`organization_id` to support multi-team scenarios.
- **macOS distribution postflight hook**: Added Cask postflight logic to auto-remove quarantine attributes.
- **Release process templates/scripts**: Added release checklist/docs and helper scripts for preflight validation and checksum generation.

### Changed
- **Unified switch flow (overview -> default instance)**: Antigravity/Codex/GitHub Copilot overview switching now follows default-instance startup logic (PID-targeted close -> inject -> start).
- **GitHub Copilot flow alignment**: Overview switching and multi-instance startup now share the same injection/start semantics.
- **Instance lifecycle alignment**: Unified start/stop/close behavior across Antigravity/Codex/VS Code with managed-directory matching and PID tracking.
- **Windows VS Code launch strategy**: Switched to `cmd /C code` for `.cmd` wrapper compatibility.
- **PID resolution semantics alignment**: VS Code PID resolving/focus now uses `Option<&str>` semantics (`None` => default instance), matching Antigravity behavior and reducing default-instance mismatch edge cases.
- **Docs and settings guidance**: Updated README/security/settings guidance for new switching and path behaviors.
- **Localization synchronization**: Updated locale keys across all supported languages for Copilot switching, open-window action, privacy mode, and related error messages.

### Fixed
- **Error compatibility and messaging**: Improved non-success status handling paths and user-facing error propagation for refresh/switch operations.
- **PR review follow-ups**: Improved error handling, added SQLite transaction safeguards in injection flow, and fixed branding inconsistencies.
- **Build hygiene**: Cleaned Windows-specific warnings and removed/quieted stale dead-code warnings.

### Removed
- **Deprecated Copilot injection entrypoint**: Removed unused legacy wrapper in favor of the unified instance-based switching pipeline.

---
## [0.6.0] - 2026-02-08

### Added
- **GitHub Copilot account management**: OAuth/Token/JSON import, quota status, plan badges, tags, batch actions, and account overview UI.
- **GitHub Copilot multi-instance**: Manage VS Code Copilot instances with isolated profiles, settings, and lifecycle actions.

### Changed
- **Dashboard & navigation**: Added GitHub Copilot entry and overview panel alongside Antigravity/Codex.
- **App-path behavior**: Rolled back the recent app-path re-detect changes to restore the previous detection flow.

### Fixed
- **Windows build warnings**: Tightened platform-specific process helpers and avoided moved environment values.

---
## [0.5.4] - 2026-02-07

### Added
- **Codex OAuth login session API**: Added command set `codex_oauth_login_start` / `codex_oauth_login_completed` / `codex_oauth_login_cancel` with `loginId + authUrl` response model.
- **OAuth timeout event contract**: Added backend timeout event payload (`loginId`, `callbackUrl`, `timeoutSeconds`) for frontend-driven retry UX.

### Changed
- **Codex OAuth flow alignment**: Switched from code-push completion to login-session completion (backend stores callback code by session, frontend completes by `loginId`).
- **UI authorization flow**: OAuth link is prepared and shown in modal first; browser open remains explicit user action.
- **Timeout retry UX**: On timeout, the main OAuth CTA switches to `Refresh authorization link`; after refresh succeeds, it switches back to `Open in Browser`.
- **Timeout behavior**: Timeout no longer triggers automatic authorization re-creation loops; retry is user-triggered.
- **OAuth observability**: Refined OAuth logs to concise operational checkpoints (session creation/start/timeout/cancel/complete), removing verbose full-payload noise.

### Removed
- **Legacy Codex OAuth commands**: Removed `prepare_codex_oauth_url`, `complete_codex_oauth`, `cancel_codex_oauth` and related frontend/service fallback paths.

### Fixed
- **Duplicate callback completion risk**: Hardened frontend callback handling with session and in-flight guards to reduce duplicate-complete races.
- **OAuth timeout UI duplication**: Resolved repeated timeout error presentation in modal by consolidating timeout-state rendering.

---
## [0.5.3] - 2026-02-06

### Added
- **Blank instance initialization mode**: Added a new initialization option when creating instances (`Copy source instance` / `Blank instance`) so users can create an empty directory without copying profile data.
- **Uninitialized-instance guide modal**: Clicking account binding on an uninitialized blank instance now opens a guide modal with a **Start now** action.
- **Instance sorting controls**: Added sort field selection (`Creation time` / `Launch time`) and ascending/descending toggle in the multi-instance toolbar.
- **In-app delete confirmation modal**: Instance deletion now uses an internal modal (with top-right close action) instead of relying on the system dialog.

### Changed
- **Instance status model**: Added `initialized` to Antigravity/Codex instance view payloads and wired it through frontend state.
- **Binding safety checks**: Binding is now blocked for uninitialized instances (disabled UI + backend validation with explicit error).
- **Instance list layout**: Status is shown in a dedicated column next to instance name; actions column is now sticky/opaque so it stays visible on narrow windows without content bleed-through.
- **Dropdown rendering split**: Inline list account dropdown renders via portal (outside container), while modal dropdown keeps in-container rendering to avoid clipping and style conflicts.
- **PID visibility rule**: PID is hidden when an instance is not running.
- **Post-start delayed refresh**: Added delayed refresh (~2s) after start to reduce stale `pending initialization` state after first boot.
- **i18n alignment**: Added and synchronized new instance-flow keys across all 17 locale files.

### Fixed
- **Delete-confirm freeze**: Fixed a scenario where delete confirmation actions could become unresponsive.

---
## [0.5.2] - 2026-02-06

### Changed
- **Account switch binding sync**: When switching Antigravity account, default instance binding now updates automatically to the selected account.
- **Codex account switch binding sync**: When switching Codex account, default Codex instance binding now updates automatically to the selected account.
- **Instance account dropdown interaction**: Inline account dropdown now uses unified open-state control so only one instance dropdown is open at a time.
- **Instances page UI polish**: Refined list/table layout, inline account selector readability, and dark mode/responsive presentation.

## [0.5.1] - 2026-02-05

### Added
- **Wakeup scheduler backend sync**: Added scheduler sync command and backend-side history load/clear APIs.
- **Download directory helper**: Exposed a system API to resolve the downloads directory.
- **App path management**: Added Codex app path to general settings and introduced app-path detect/set commands.

### Changed
- **Wakeup history storage**: Moved history persistence to backend storage with higher retention (up to 100 items).
- **macOS launch strategy**: Prefer direct executable launch (PID available), fallback to `open -a` for `.app` paths.
- **App path reset**: Reset now auto-detects and fills the path instead of clearing it.
- **Account switching**: Update default instance PID after launch; emit app-path-missing events when needed.
- **Documentation**: Added multi-instance sections and image placeholders for Antigravity/Codex.
- **i18n**: Added new app-path related keys and ensured locale consistency.

### Fixed
- **macOS app selection**: Improved `.app` selection/launch flow to reduce permission errors.

## [0.5.0] - 2026-02-04

### Added
- **Antigravity Latest Version Compatibility**: Enhanced account switching support for Antigravity 1.16.5+.
  - Support for new unified state sync format (`antigravityUnifiedStateSync.oauthToken`).
  - Backward compatible with legacy format for older versions.
- **Antigravity Multi-Instance Support**: Run multiple Antigravity IDE instances simultaneously.
  - Each instance runs with an isolated user profile and data directory.
  - Support for different accounts logged in to different instances concurrently.
  - Create, launch, restart, and delete instances with a dedicated management interface.
  - Auto-detect running instances and display their status in real-time.
- **Codex Desktop Multi-Instance Support**: Run multiple Codex desktop instances simultaneously on macOS.
  - Each instance runs with an isolated user profile and app data directory.
  - Support for different accounts logged in to different instances concurrently.
  - Create, launch, restart, and delete instances with a dedicated management interface.
  - Auto-detect running instances and display their status in real-time.
  - Smart restart strategy: choose between "Always Restart", "Never Restart", or "Ask Me" when switching accounts.

### Changed
- **Instance Management UI**: New dedicated instance management page with modern list-based interface.
- **Navigation**: Added "Instances" menu item to sidebar for quick access to instance management.

---
## [0.4.10] - 2026-01-31

### Changed
- **Single account quota refresh**: Single card refresh now always fetches from the real-time API, bypassing the 60-second cache.
- **Cache directory isolation**: Desktop quota cache moved to `quota_api_v1_desktop` to prevent sharing/overwriting with the extension.

## [0.4.9] - 2026-01-31

### Added
- **Quota error details**: Store the last quota error per account and show it in a dedicated error details modal (with link rendering).
- **Forbidden status UI**: Show 403 forbidden status with a lock badge and an in-place quota banner.

### Changed
- **Quota fetch results**: Return structured error info (code/message) and persist it into account state.
- **Account status hints**: Combine disabled/warning/forbidden hints in tooltips.
- **Account actions UI**: Tightened action button spacing and size for account cards.

### Fixed
- **i18n**: Filled missing translations for account error actions and error detail fields.

## [0.4.8] - 2026-01-30

### Added
- **OpenCode sync toggle**: Add a switch in Codex account management to control OpenCode sync/restart.

### Changed
- **OpenCode auth sync**: Sync OpenCode auth.json on account switch with full OAuth fields and platform-aware path.
- **OpenCode restart**: Start OpenCode when not running; restart when running.
- **AccountId alignment**: Align account_id extraction with the official extension (access_token only).
- **UI copy**: Settings OpenCode path hint now generic without a hardcoded default path.

### Fixed
- **i18n**: Filled missing translations and ensured locale keys are consistent across languages.

## [0.4.7] - 2026-01-30

### Added
- **Authorized API cache**: Cache raw authorized API responses in `cache/quota_api_v1`.
- **Cache source marker**: Store `customSource` in API cache records to identify the writer.
- **Cache hit logging**: Log API cache hits/expiry during quota refresh.

### Changed
- **Legacy cache reader**: Reads the new API cache payload to preserve fast startup behavior.

## [0.4.6] - 2026-01-29

### Added
- **Update Notification**: Update dialog now displays release notes with localized content (English/Chinese).

### Fixed
- **i18n**: Fixed missing translations in Codex add account modal (OAuth, Token, Import tabs).
- **Accessibility**: Improved FREE tier badge contrast for better readability in light mode.
- **i18n**: Fixed hardcoded Chinese strings in tag deletion confirmation dialog.

---
## [0.4.3] - 2026-01-29

### Added
- **Codex Tag Management**: Added global tag deletion for Codex accounts.
- **Account Filtering & Tagging**:
  - Support for managing account tags (add/remove).
  - Support for filtering accounts by tags.
- **Compact View**:
  - Added compact view mode for account list.
  - Added status icons for disabled or warning states in compact view.
  - Support customizable model grouping in compact view.

### Changed
- **Smart Recommendations**: Improved dashboard recommendation logic to exclude disabled, forbidden, or empty accounts.
- **UI Improvements**:
  - Refined compact view interactions.
  - Removed redundant tag rendering in list views.
  
## [0.4.2] - 2026-01-29

### Added
- **Update Modal**: Unified update check into a modal dialog, including the entry in Settings → About.
- **Refresh Frequency**: Added Codex auto refresh interval settings (default 10 minutes).
- **Account Warnings**: Show refresh warnings in the account list, including invalid-credential hints.

### Changed
- **Update UX**: Update prompt now uses a non-transparent modal consistent with existing dialogs.

## [0.4.1] - 2026-01-29

### Added
- **Close Confirmation**: New close dialog with minimize/quit actions and a “remember choice” option.
- **Close Behavior Setting**: Configure the default close action in Settings → General.
- **Tray Menu**: System tray menu with navigation shortcuts and quota refresh actions.
- **Sorting Enhancements**: Sort by reset time for Antigravity group quotas and Codex weekly/hourly quotas.

### Changed
- **i18n**: Updated translations for close dialog, close behavior, and reset-time sorting across all 17 languages.
- **UI Polish**: Refined styling to support the new close dialog and related layout updates.


## [0.4.0] - 2026-01-28

### Added
- **Visual Dashboard**: Brand new dashboard providing a one-stop overview of both Antigravity and Codex accounts status.
- **Codex Support**: Full support for Codex account management.
  - View Hourly (5H) and Weekly quotas.
  - Automatic Plan recognition (Basic, Plus, Team, Enterprise).
  - Independent account list and card view.
- **Rebranding**: Project officially renamed to **Cockpit Tools**.
- **Sponsor & Feedback**: Added "Sponsor" and "Feedback" sections in Settings -> About for better community engagement.

### Changed
- **UI Overhaul**: Redesigned dashboard cards for extreme compactness and symmetry.
- **Typography**: switched default font to **Inter** for better readability.
- **Documentation**: Comprehensive update to README with fresh screenshots and structured feature overview.
- **i18n**: Updated translations for all 17 languages to cover new Dashboard and Codex features.


## [0.3.3] - 2026-01-24

### Added
- **Account Management**: Added sorting by creation time. Accounts are now sorted by creation time (descending) by default.
- **Database**: Added `created_at` field to the `accounts` table for precise account tracking.
- **i18n**: Added "Creation Time" related translations for all 17 supported languages.

## [0.3.2] - 2026-01-23

### Added
- **Engineering**: Added automatic version synchronization script. `package.json` version now automatically syncs to `tauri.conf.json` and `Cargo.toml`.
- **Engineering**: Added git pre-commit hook to strictly enforce Changelog updates when version changes.

## [0.3.1] - 2026-01-23

### Changed
- **Maintenance**: Routine version update and dependency maintenance.

## [0.3.0] - 2026-01-22

### Added
- **Model Grouping Management**: New grouping modal to customize model group display names.
  - Four fixed groups: Claude 4.5, G3-Pro, G3-Flash, G3-Image.
  - Custom group names are applied to account cards and sorting dropdowns.
  - Group settings are persisted locally and auto-initialized on first launch.
- **Account Sorting**: Added sorting options for account list.
  - Default sorting by overall quota.
  - Sort by specific group quota (e.g., by Claude 4.5 quota).
  - Secondary sorting by overall quota when group quotas are equal.
- **i18n**: Added sorting and group management translations for all 17 supported languages.

### Changed
- Model names on account cards now dynamically reflect custom group names.
- Removed "Other" group display to simplify the grouping model.
- Decoupled grouping configuration between desktop app and VS Code extension.

---

## [0.2.0] - 2026-01-21

### Added
- **Update Checker**: Implemented automatic update checking via GitHub Releases API.
  - On startup, the app checks for new versions (once every 24 hours by default).
  - A beautiful glassmorphism notification card appears in the top-right corner when an update is available.
  - Manual "Check for Updates" button added to **Settings → About** page with real-time status feedback.
  - Clicking the notification opens the GitHub release page for download.
- **i18n**: Added update notification translations for all 17 supported languages.

---

## [0.1.0] - 2025-01-21

### Added
- **Account Management**: Complete account management with OAuth authorization support.
  - Add accounts via Google OAuth authorization flow.
  - Import accounts from Antigravity Tools (`~/.antigravity_tools/`), local Antigravity client, or VS Code extension.
  - Export accounts to JSON for backup and migration.
  - Delete single or multiple accounts with confirmation.
  - Drag-and-drop reordering of account list.
- **Quota Monitoring**: Real-time monitoring of model quotas for all accounts.
  - Card view and list view display modes.
  - Filter accounts by subscription tier (PRO/ULTRA/FREE).
  - Auto-refresh with configurable intervals (2/5/10/15 minutes or disabled).
  - Quick switch between accounts with one click.
- **Device Fingerprints**: Comprehensive device fingerprint management.
  - Generate new fingerprints with customizable names.
  - Capture current device fingerprint.
  - Bind fingerprints to accounts for device simulation.
  - Import fingerprints from Antigravity Tools or JSON files.
  - Preview fingerprint profile details.
- **Wakeup Tasks**: Automated account wakeup scheduling system.
  - Create multiple wakeup tasks with independent controls.
  - Supports scheduled, Crontab, and quota-reset trigger modes.
  - Multi-model and multi-account selection.
  - Custom wakeup prompts and max token limits.
  - Trigger history with detailed logs.
  - Global wakeup toggle for quick enable/disable.
- **Antigravity Cockpit Integration**: Deep integration with the VS Code extension.
  - WebSocket server for bidirectional communication.
  - Remote account switching from the extension.
  - Account import/export synchronization.
- **Settings**: Comprehensive application settings.
  - Language selection (17 languages supported).
  - Theme switching (Light/Dark/System).
  - WebSocket service configuration with custom port support.
  - Data and fingerprint directory shortcuts.
- **i18n**: Full internationalization support for 17 languages.
  - 🇨🇳 简体中文, 🇹🇼 繁體中文, 🇺🇸 English
  - 🇯🇵 日本語, 🇰🇷 한국어, 🇻🇳 Tiếng Việt
  - 🇩🇪 Deutsch, 🇫🇷 Français, 🇪🇸 Español, 🇮🇹 Italiano, 🇵🇹 Português
  - 🇷🇺 Русский, 🇹🇷 Türkçe, 🇵🇱 Polski, 🇨🇿 Čeština, 🇸🇦 العربية
- **UI/UX**: Modern, polished user interface.
  - Glassmorphism design with smooth animations.
  - Responsive sidebar navigation.
  - Dark mode support with seamless theme transitions.
  - Native macOS window controls and drag region.

### Technical
- Built with Tauri 2.0 + React + TypeScript.
- SQLite database for local data persistence.
- Secure credential storage using system keychain.
- Cross-platform support (macOS primary, Windows/Linux planned).

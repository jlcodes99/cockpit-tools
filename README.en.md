# Cockpit Tools

English · [简体中文](README.md)

[![GitHub stars](https://img.shields.io/github/stars/jlcodes99/cockpit-tools?style=flat&color=gold)](https://github.com/jlcodes99/cockpit-tools)
[![GitHub downloads](https://img.shields.io/github/downloads/jlcodes99/cockpit-tools/total?style=flat&color=blue)](https://github.com/jlcodes99/cockpit-tools/releases)
[![GitHub release](https://img.shields.io/github/v/release/jlcodes99/cockpit-tools?style=flat)](https://github.com/jlcodes99/cockpit-tools/releases)
[![GitHub issues](https://img.shields.io/github/issues/jlcodes99/cockpit-tools)](https://github.com/jlcodes99/cockpit-tools/issues)

A **universal AI IDE account management tool**, currently supporting **Antigravity**, **Codex**, **GitHub Copilot**, **Windsurf**, **Kiro**, **Cursor**, **Gemini Cli**, **CodeBuddy**, **CodeBuddy CN**, **Qoder**, **Trae**, and **Zed**, with multi-instance parallel workflows.

> Designed to help users efficiently manage multiple AI IDE accounts, this tool supports one-click switching, quota monitoring, wake-up tasks, and multi-instance parallel runs, helping you fully utilize resources from different accounts.

**Features**: One-click Switch · Multi-account Management · Multi-instance · Quota Monitoring · Wake-up Tasks · Device Fingerprints · Plugin Integration · GitHub Copilot Management · Windsurf Management · Kiro Management · Cursor Management · Gemini Cli Management · CodeBuddy Management · CodeBuddy CN Management · Qoder Management · Trae Management · Zed Management

**Languages**: Supports 18 languages

🇺🇸 English · 🇨🇳 简体中文 · 繁體中文 · 🇯🇵 日本語 · 🇩🇪 Deutsch · 🇪🇸 Español · 🇫🇷 Français · 🇮🇹 Italiano · 🇰🇷 한국어 · 🇧🇷 Português · 🇷🇺 Русский · 🇹🇷 Türkçe · 🇵🇱 Polski · 🇨🇿 Čeština · 🇸🇦 العربية · 🇻🇳 Tiếng Việt · 🇮🇩 Bahasa Indonesia

---

## Feature Overview

### 1. Dashboard

A brand new visual dashboard providing a one-stop status overview:

- **Twelve-Platform Support**: Simultaneously displays Antigravity, Codex, GitHub Copilot, Windsurf, Kiro, Cursor, Gemini Cli, CodeBuddy, CodeBuddy CN, Qoder, Trae, and Zed account status
- **Quota Monitoring**: Real-time view of remaining quotas and reset times for each model
- **Quick Actions**: One-click refresh, one-click wake-up
- **Visual Progress**: Intuitive progress bars showing quota consumption

> ![Dashboard Overview](docs/images/dashboard_overview.png)

### 2. Antigravity Account Management

- **One-Click Switch**: Switch the currently active account instantly without manual login/logout
- **Multiple Import Methods**: OAuth, Refresh Token, Plugin Sync
- **Wake-up Tasks**: Schedule AI model wake-ups to trigger quota reset cycles in advance
- **Device Fingerprints**: Generate, manage, and bind device fingerprints to reduce risk

> ![Antigravity Accounts](docs/images/antigravity_list.png)
>
> *(Wakeup Tasks & Device Fingerprints)*
> ![Wakeup Tasks](docs/images/wakeup_detail.png)
> ![Device Fingerprints](docs/images/fingerprint_detail.png)

#### 2.1 Antigravity Multi-Instance

Run multiple Antigravity instances in parallel with different accounts. For example, open two Antigravity instances, bind different accounts, and handle different projects independently.

- **Isolated Accounts**: Each instance binds a different account and runs independently
- **Parallel Projects**: Run multiple tasks/projects at the same time
- **Argument Isolation**: Custom instance directory and launch arguments

> ![Antigravity Instances](docs/images/antigravity_instances.png)

### 3. Codex Account Management

- **Dedicated Support**: Optimized account management experience for Codex
- **Quota Display**: Clear display of Hourly and Weekly quota status
- **Plan Recognition**: Automatically identifies account Plan types (Basic, Plus, Team, etc.)

> ![Codex Accounts](docs/images/codex_list.png)

#### 3.1 Codex Multi-Instance

Codex also supports parallel multi-instance usage. For example, open two Codex instances, bind different accounts, and handle different projects independently.

- **Isolated Accounts**: Each instance binds a different account and runs independently
- **Parallel Projects**: Run multiple tasks/projects at the same time
- **Argument Isolation**: Custom instance directory and launch arguments

> ![Codex Instances](docs/images/codex_instances.png)

### 4. GitHub Copilot Account Management

- **Account Import**: OAuth, Token/JSON import
- **Quota View**: Inline Suggestions / Chat messages usage and reset time
- **Plan Recognition**: Auto-detects Free / Individual / Pro / Business / Enterprise tiers
- **Batch Operations**: Tags and bulk actions

#### 4.1 GitHub Copilot Multi-Instance

Manage VS Code Copilot instances with isolated profiles and lifecycle controls.

- **Isolated Profiles**: Each instance uses its own user data directory
- **Quick Lifecycle**: Start/stop/force stop instances
- **Window Control**: Open instance windows and close all instances

### 5. Windsurf Account Management

- **Account Import**: OAuth, Token/JSON import, and local import
- **Quota View**: Shows Plan, User Prompt credits, Add-on prompt credits, and cycle information
- **Batch Operations**: Tags and bulk actions
- **Switch Injection**: Supports injecting and launching Windsurf after account switch

#### 5.1 Windsurf Multi-Instance

Manage Windsurf instances with isolated profiles and lifecycle controls.

- **Isolated Profiles**: Each instance uses its own user data directory
- **Quick Lifecycle**: Start/stop/force stop instances
- **Window Control**: Open instance windows and close all instances

### 6. Kiro Account Management

- **Account Import**: OAuth, Token/JSON import, and local import
- **Quota View**: Shows Plan, User Prompt credits, Add-on prompt credits, and cycle information
- **Batch Operations**: Tags and bulk actions
- **Switch Injection**: Supports injecting and launching Kiro after account switch

#### 6.1 Kiro Multi-Instance

Manage Kiro instances with isolated profiles and lifecycle controls.

- **Isolated Profiles**: Each instance uses its own user data directory
- **Quick Lifecycle**: Start/stop/force stop instances
- **Window Control**: Open instance windows and close all instances

### 7. Cursor Account Management

- **Account Import**: OAuth, Token/JSON import, and local import
- **Quota View**: Shows Total Usage, Auto + Composer, API Usage, On-Demand, and cycle information
- **Batch Operations**: Tags and bulk actions
- **Switch Injection**: Supports injecting and launching Cursor after account switch

#### 7.1 Cursor Multi-Instance

Manage Cursor instances with isolated profiles and lifecycle controls.

- **Isolated Profiles**: Each instance uses its own user data directory
- **Quick Lifecycle**: Start/stop/force stop instances
- **Window Control**: Open instance windows and close all instances

### 8. Gemini Cli Account Management

- **Account Import**: OAuth, Token/JSON import, and local import
- **Quota View**: Shows Total Usage, Auto + Composer, API Usage, On-Demand, and cycle information
- **Batch Operations**: Tags and bulk actions
- **Switch Injection**: Supports injecting Gemini Cli local credentials after account switch (`~/.gemini`)
- **Platform Limitation**: Gemini Cli multi-instance management is not supported yet

### 9. CodeBuddy Account Management

- **Account Import**: OAuth and Token/JSON import
- **Quota View**: quota query, cycle details, and extra-credit display
- **Batch Operations**: tags and bulk actions
- **Switch Injection**: supports injecting and launching CodeBuddy after account switch

#### 9.1 CodeBuddy Multi-Instance

Manage CodeBuddy instances with isolated profiles and lifecycle controls.

- **Isolated Profiles**: Each instance uses its own user data directory
- **Quick Lifecycle**: Start/stop/force stop instances
- **Window Control**: Open instance windows and close all instances

### 10. CodeBuddy CN Account Management

- **Account Import**: supports OAuth, Token/JSON import, and local-client import
- **Quota View**: shows plan and usage status, with a shortcut to open detailed quota information on the official web page
- **Batch Operations**: supports tags and bulk actions
- **Switch Injection**: supports writing local auth state back and launching CodeBuddy CN after account switch

#### 10.1 CodeBuddy CN Multi-Instance

Manage CodeBuddy CN instances with isolated profiles and lifecycle controls.

- **Isolated Profiles**: each instance uses its own user data directory
- **Quick Lifecycle**: start/stop/force stop instances
- **Window Control**: open instance windows and close all instances

### 11. Qoder Account Management

- **Account Import**: supports local import and JSON import
- **Quota View**: shows Credits usage, remaining credits, and raw plan values
- **Batch Operations**: supports tags, filters, export, and batch delete/refresh
- **Switch Injection**: supports injecting and launching Qoder after account switch

#### 11.1 Qoder Multi-Instance

Manage Qoder instances with isolated profiles and lifecycle controls.

- **Isolated Profiles**: each instance uses its own user data directory
- **Quick Lifecycle**: start/stop/force stop instances
- **Window Control**: open instance windows and close all instances

### 12. Trae Account Management

- **Account Import**: supports local import and JSON import
- **Quota View**: shows raw plan values, USD spent/total budget, and reset time
- **Batch Operations**: supports tags, filters, export, and batch delete/refresh
- **Switch Injection**: supports writing back local auth state and launching Trae after account switch

#### 12.1 Trae Multi-Instance

Manage Trae instances with isolated profiles and lifecycle controls.

- **Isolated Profiles**: each instance uses its own user data directory
- **Quick Lifecycle**: start/stop/force stop instances
- **Window Control**: open instance windows and close all instances

### 13. Zed Account Management

- **Account Import**: Supports official OAuth sign-in, JSON import, and importing the current local sign-in state
- **Usage View**: Shows subscription status, Edit Predictions, Token Spend, Spend Limit, and billing period end
- **Batch Operations**: Supports tags, filters, export, and batch delete/refresh
- **Switch Injection**: Applies the selected account back to the official Zed client using the client's real local persistence rules and restarts the client when needed

### 14. General Settings

- **Personalized Settings**: Theme switching, language settings, auto-refresh interval
- **Platform Controls**: Centralized CodeBuddy CN/Qoder/Trae/Zed launch-path and quota-alert settings

> ![Settings](docs/images/settings_page.png)

---

## Security & Privacy (Plain-English)

These are the most common security questions answered directly:

- **This is a local desktop tool**: it does not require a separate cloud account for this project, and it does not rely on a project-hosted cloud account storage.
- **Data is mainly stored on your machine**:
  - `~/.antigravity_cockpit`: Antigravity accounts, configs, WebSocket status, etc.
  - `~/.codex`: official Codex current login `auth.json`
  - `~/.gemini`: Gemini Cli local session files (for example `oauth_creds.json`, `google_accounts.json`, `settings.json`)
  - local app data folder under `com.antigravity.cockpit-tools`: Codex / GitHub Copilot / Windsurf / Kiro / Cursor / Gemini Cli / CodeBuddy / CodeBuddy CN / Qoder / Trae / Zed multi-account index data, etc.
- **WebSocket is local-only by default**: binds to `127.0.0.1`, default port `19528`; you can disable it or change the port in Settings.
- **When network access happens**: OAuth login, token refresh, quota fetching, update checks, and other official API requests.
- **Practical safety tips**:
  1. If you do not need plugin integration, disable WebSocket.
  2. Do not share your full user directory directly; redact token files before backup/share.
  3. On shared/public computers, remove accounts and quit the app after use.

## Settings Guide (Beginner Friendly)

If you want a stable setup with minimal tuning, follow the "Recommended" values.

### General Settings

| Setting | What it does (simple) | Recommended | When to change |
| --- | --- | --- | --- |
| Display Language | Changes UI language | Your native/comfortable language | Only if current language is hard to read |
| Theme | Light/dark appearance | System | Use dark mode for long night sessions |
| Window Close Behavior | What happens when clicking close | Ask every time | Choose "Minimize to tray" if you want background running |
| Antigravity Auto Refresh | Periodically updates Antigravity quota | 5-10 minutes | Use 2 minutes if you need near real-time updates |
| Codex Auto Refresh | Periodically updates Codex quota | 5-10 minutes | Same as above |
| GitHub Copilot Auto Refresh | Periodically updates GitHub Copilot quota | 5-10 minutes | Same as above |
| Windsurf Auto Refresh | Periodically updates Windsurf quota | 5-10 minutes | Same as above |
| Kiro Auto Refresh | Periodically updates Kiro quota | 5-10 minutes | Same as above |
| Cursor Auto Refresh | Periodically updates Cursor quota | 5-10 minutes | Same as above |
| Gemini Cli Auto Refresh | Periodically updates Gemini Cli quota | 5-10 minutes | Same as above |
| CodeBuddy Auto Refresh | Periodically updates CodeBuddy quota | 5-10 minutes | Same as above |
| CodeBuddy CN Auto Refresh | Periodically updates CodeBuddy CN quota | 5-10 minutes | Same as above |
| Qoder Auto Refresh | Periodically updates Qoder quota | 5-10 minutes | Same as above |
| Trae Auto Refresh | Periodically updates Trae quota | 5-10 minutes | Same as above |
| Zed Auto Refresh | Periodically updates Zed quota | 5-10 minutes | Same as above |
| Data Directory | Where account/config files are stored | Keep default | Only for troubleshooting or backups |
| Antigravity/Codex/VS Code/Windsurf/Kiro/Cursor/Gemini Cli/CodeBuddy/CodeBuddy CN/Qoder/Trae/Zed/OpenCode App Path | Manually set executable path | Leave empty (auto-detect) | Change only if auto-detect fails or you use custom install paths |
| Auto-restart OpenCode on Codex switch | Sync OpenCode auth after Codex switch | ON if you use OpenCode; otherwise OFF | Enable for frequent Codex switching with OpenCode |

Notes:
- Smaller refresh intervals mean more frequent requests.
- If quota-reset wake-up tasks are enabled, some minimum refresh limits may apply (UI will show hints).

### Network Settings

| Setting | What it does (simple) | Recommended | Risk / Notes |
| --- | --- | --- | --- |
| WebSocket Service | Real-time local integration for plugins/clients | OFF if not needed | Still local-only (`127.0.0.1`) when enabled |
| Preferred Port | Listening port for WebSocket | Default `19528` | Change only on conflict; restart required after save |
| Current Running Port | The actual active port | Read-only info | May differ if preferred port is occupied |

### 3 Ready-to-Use Presets

1. **Stable default**: 10-min refresh, WebSocket OFF (if no plugin), keep default paths.  
2. **Frequent switching**: 2-5 min refresh, WebSocket ON if needed, OpenCode sync ON.  
3. **Security-first**: WebSocket OFF, do not share user directory, remove unused accounts regularly.  

---



---

## Installation Guide

### Option A: Manual Download (Recommended)

Go to [GitHub Releases](https://github.com/jlcodes99/cockpit-tools/releases) to download the package for your system:

*   **macOS**: `.dmg` (Apple Silicon & Intel)
*   **Windows**: `.msi` (Recommended) or `.exe`
*   **Linux**: `.deb` (Debian/Ubuntu) or `.AppImage` (Universal)

### Option B: Install with Homebrew (macOS)

> Homebrew is required.

```bash
brew tap jlcodes99/cockpit-tools https://github.com/jlcodes99/cockpit-tools
brew install --cask cockpit-tools
```

If you hit the macOS "App is damaged" warning, you can also install with `--no-quarantine`:

```bash
brew install --cask --no-quarantine cockpit-tools
```

If Homebrew says the app already exists (e.g. `already an App at '/Applications/Cockpit Tools.app'`), remove the old app and install again:

```bash
rm -rf "/Applications/Cockpit Tools.app"
brew install --cask cockpit-tools
```

Or force overwrite the existing app:

```bash
brew install --cask --force cockpit-tools
```

### Option C: Arch Linux (AUR)

Install via an AUR helper such as `yay` or `paru`:

```bash
# Source-built package
yay -S cockpit-tools

# Prebuilt binary package
yay -S cockpit-tools-bin
```

### 🛠️ Troubleshooting

#### macOS says "App is damaged and can't be opened"?
Due to macOS security mechanisms, apps not downloaded from the App Store may trigger this warning. You can quickly fix this by following these steps:

1.  **Command Line Fix** (Recommended):
    Open Terminal and run the following command:
    ```bash
    sudo xattr -rd com.apple.quarantine "/Applications/Cockpit Tools.app"
    ```
    > **Note**: If you changed the app name, please adjust the path in the command accordingly.

2.  **Or**: Go to "System Settings" -> "Privacy & Security" and click "Open Anyway".

---

## Development & Build

### Prerequisites

- Node.js v18+
- npm v9+
- Rust (Tauri runtime)

### Install Dependencies

```bash
npm install
```

### Development Mode

```bash
npm run tauri dev
```

### Build

```bash
npm run tauri build
```

---

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=jlcodes99/cockpit-tools&type=Date)](https://star-history.com/#jlcodes99/cockpit-tools&Date)

---

## Sponsor

If you find this project useful, consider supporting it here: [☕ Donate](docs/DONATE.en.md)

Every bit of support helps sustain open-source development. Thank you!

---

## Acknowledgments

- Antigravity account switching logic based on: [Antigravity-Manager](https://github.com/lbjlaq/Antigravity-Manager)

Thanks to the project author for their open-source contributions! If these projects have helped you, please give them a ⭐ Star to show your support!

---

## License

This project is licensed under [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/).

- Allowed: personal learning, research, and non-commercial use/modification (with attribution and share-alike obligations).
- Not allowed: any commercial use without authorization (including internal commercial operations, external paid services, paid product integration, or resale/redistribution for profit).
- Commercial license: contact the author for a separate written commercial license and pricing.

---

## Disclaimer

This project is for personal learning and research purposes only. By using this project, you agree to:

- Not use this project for any commercial purposes without prior written authorization from the author
- Bear all risks and responsibilities of using this project
- Comply with relevant terms of service and laws and regulations

The project author is not responsible for any direct or indirect losses arising from the use of this project.

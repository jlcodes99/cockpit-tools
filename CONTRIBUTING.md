# Contributing to Cockpit Tools

Thank you for your interest in contributing to Cockpit Tools! This project aims to be the universal manager for AI IDEs, and we welcome contributions of all kinds.

## 🚀 Getting Started

1.  **Fork** the repository on GitHub.
2.  **Clone** your fork locally:
    ```bash
    git clone https://github.com/YOUR_USERNAME/cockpit-tools.git
    ```
3.  **Create a branch** for your feature or bug fix:
    ```bash
    git checkout -b feature/my-cool-feature
    ```

## 🛠️ Project Structure

This project is a Cargo Workspace:
- `crates/cockpit-core`: Shared business logic (Library).
- `src-tauri`: The GUI application (Tauri + React).
- `crates/cockpit-cli`: The command-line interface.

## 📝 Coding Standards

- **Rust:** Follow standard Rust idioms. Run `cargo fmt` before committing.
- **Frontend:** We use React 19 and Tailwind CSS. Use functional components and hooks.
- **Commits:** Use clear, descriptive commit messages.

## 🧪 Testing

- **GUI:** `npm run tauri dev`
- **CLI:** `cargo run --package cockpit-cli -- <commands>`
- **Core:** `cargo test --package cockpit-core`

## 📬 Submitting a Pull Request

1.  Push your changes to your fork.
2.  Open a Pull Request against the `main` branch.
3.  Provide a clear description of the changes and link any related issues.
4.  Be prepared to iterate based on feedback!

## 📜 Code of Conduct

Please be respectful and professional in all interactions. We follow the [Contributor Covenant](https://www.contributor-covenant.org/).

---

## 📋 Additional Project Specifications

### TypeScript Configuration

- **Strict mode** enabled (`strict: true`)
- **No unused locals** (`noUnusedLocals: true`)
- **No unused parameters** (`noUnusedParameters: true`)
- **No fallthrough cases in switch** (`noFallthroughCasesInSwitch: true`)
- Target: ES2020, JSX: react-jsx

### Build & Development

| Command | Description |
|---------|-------------|
| `npm run tauri dev` | Start development server (port 1420) |
| `npm run typecheck` | Run TypeScript type checking (auto-runs before build) |
| `npm run build` | Build frontend (syncs version + typecheck + vite build) |
| `npm run sync-version` | Sync `package.json` version to Tauri config |
| `npm run release:preflight` | Run full release pre-check (locales + typecheck + build + cargo check) |

### State Management & i18n

- **State management:** Zustand
- **Internationalization:** i18next + react-i18next (supports 18 languages)
- **UI framework:** Tailwind CSS + DaisyUI

### Release Process Summary

1. Update version in `package.json` + CHANGELOG (Chinese & English)
2. Run `npm run sync-version` → `npm run release:preflight`
3. Commit → Tag (e.g., `v0.22.20`) → Push branch and tag
4. CI auto-builds macOS (universal) + Windows + Linux → Generates SHA256 → Updates Homebrew Cask

### Release Targets

- **macOS, Windows, and Linux**
- macOS recommended: `universal.dmg` (compatible with both Intel and Apple Silicon)
- Linux packages: `.AppImage`, `.deb`, and `.rpm`
- Each release includes `SHA256SUMS.txt` for integrity verification

### Branch Strategy

- **Main branch:** `main`
- **PR target:** `main`
- **Release tag format:** `v<major>.<minor>.<patch>`

### Related Specification Files

| File | Content |
|------|---------|
| `docs/release-process.md` | Detailed release process documentation |
| `tsconfig.json` | TypeScript strict compilation rules |
| `.github/workflows/release.yml` | CI/CD release automation |
| `SECURITY.md` | Security policy |

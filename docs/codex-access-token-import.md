# Codex access token account import

## Purpose

Cockpit Tools supports adding a Codex account with only an account name and a Codex access token. The imported account is stored as a Codex OAuth account, not as an API key account.

## Behavior

- Open the Codex accounts page and choose the access token import tab.
- Enter an account name and paste the Codex access token.
- Cockpit saves the account with `auth_mode = OAuth`.
- When switching an access-token-only account into a Codex home, Cockpit writes the official personal access token auth shape: `OPENAI_API_KEY = null` plus `personal_access_token`.
- No API key auth file is created for this flow.
- After import, Cockpit attempts to refresh the account profile and quota. If the token is expired or rejected, the account remains saved and the quota/profile error is shown in the normal Codex account UI.

## Launching Codex

- Use the Codex instances page to bind the imported account to an App-mode Codex instance and click start.
- App-mode launch uses the existing Cockpit Codex instance path, including `CODEX_HOME`, `CODEX_ELECTRON_USER_DATA_PATH`, and `--user-data-dir`.
- The CLI/terminal launch path remains separate and is used only for CLI-mode instances.

## Sessions

- Default Codex thread sync is enabled for new or missing default instance settings.
- Before App-mode launch, Cockpit runs a targeted session visibility repair for the target instance and asks the official app-server to rebuild thread metadata.
- Existing session import, export, restore, and trash management continue to use the existing Codex session manager.

## Token lifetime

Access-token-only accounts do not have a refresh token. When the access token expires, re-import or update the access token for that account.

# Use native Responses for the official DeepSeek Codex preset

## Decision

For the **official DeepSeek preset** (`api.deepseek.com`):

1. **Default protocol is Responses** (`wire_api = "responses"`), aligned with DeepSeek’s Codex setup script.
2. **Chat Completions remains available** when the user explicitly selects it (no hard lock).
3. Responses stays native, but the instance provider gateway **only rewrites the model name**:
   - Picker `display_name` is `DeepSeek-V4-Flash` / `Pro`.
   - Catalog `slug` uses official Codex whitelist shells `gpt-5.5` / `gpt-5.4`.
   - The sidecar replaces that slug with `deepseek-v4-flash` / `pro` and forwards the Responses body unchanged.
   - Cockpit writes `cockpit-provider-model-catalog.json` into the **target instance `CODEX_HOME`**.
   - Official DeepSeek tool metadata is overlaid onto the Codex client-model shape.
   - Instance `models_cache.json` is invalidated after the catalog write.
4. **Default model prefers `deepseek-v4-flash`**. Leftover shell names such as `gpt-5.6-sol` are replaced on switch.
5. Codex API Service can still keep a **visible per-account mapping** for pool rotation. That mapping is not used by desktop instance startup.

## Why

The user wants native Responses without a local remapping gateway. Official slugs plus official catalog metadata let Codex emit DeepSeek tool/shell/apply_patch shapes and send names the official API accepts.

## Consequences

- Desktop / multi-instance start uses a per-instance provider gateway sidecar only to rewrite `gpt-5.5` / `gpt-5.4` → `deepseek-v4-flash` / `pro`.
- Enabling from 模型供应商 onto a non-default instance writes `{instance_dir}/cockpit-provider-model-catalog.json`.
- Chat Completions still uses the instance provider gateway for protocol conversion.
- Existing DeepSeek accounts with explicit Chat Completions are **not** auto-migrated away from Chat.
- Accounts without a wire protocol still default to Responses for DeepSeek.

## Update: direct / CDP catalogs use the official entries verbatim

The shell-template decision above only still applies to **gateway** mode, where the instance
provider gateway rewrites the model name. The **direct** and **CDP** catalogs no longer derive
their metadata from a Codex built-in model shell:

- Entries written to `{instance_dir}/cockpit-model-catalog.json` are the complete official
  `models.json` entries, so `multi_agent_version`, `minimal_client_version`,
  `effective_context_window_percent` and friends match DeepSeek's declaration instead of the
  shell's, and shell-only fields (pricing tiers, plan gating, apps/plugins instruction switches)
  no longer leak onto a DeepSeek model.
- Switching to DeepSeek also writes `web_search = "disabled"` and temporarily removes the
  top-level keys that contradict the official model declaration (the `DEL_B` list of DeepSeek's
  `codex-deepseek-setup.sh`), recording original values so switching away restores them.
  `model_context_window` / `model_auto_compact_token_limit` are intentionally *not* in that list:
  Cockpit's context-management feature owns them as an explicit user setting.

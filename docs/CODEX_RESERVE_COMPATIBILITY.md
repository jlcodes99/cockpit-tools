# Codex Luna Reserve compatibility

## Verified source

Inspected the installed official client on 2026-09-05:

- Application: `/Applications/ChatGPT.app`
- Bundle identifier: `com.openai.codex`
- Version: `26.901.31953` (build `7868`)
- Archive: `Contents/Resources/app.asar`
- Relevant entries: `webview/assets/app-initial-caa927532ffb.js` and
  `webview/assets/app-primary-37ff25fd4643.js`
- The bundled `Contents/Resources/codex` model JSON has a Luna entry but no
  dedicated `gpt-reserve` entry. This is a bundled fallback, not an account's
  live server model catalog.

## Official behavior

`gpt-reserve` is the request ID for temporary **Luna Reserve**, not an ordinary
permanent model choice. The client requires eligible ChatGPT authentication,
matching account/user/plan data, a compatible client version, and official
feature gates (including `reserve_enabled`). Its activation predicate includes:

```javascript
const reserveAllowed = status.additional_rate_limits?.some(
  ({ limit_name, rate_limit }) =>
    limit_name === "gpt-reserve" && rate_limit?.allowed === true,
);
const active = status.rate_limit_upsell?.banner_type === "luna_reserve"
  && status.rate_limit?.allowed === false
  && reserveAllowed === true;
```

The picker looks for `gpt-reserve`, falling back to `gpt-5.6-luna`, then copies
that entry with the request model set to `gpt-reserve`. It displays the entry's
Luna name with a moon indicator. The sidebar calls the allowance "Luna Reserve".
The original model and reasoning effort are remembered and restored when the
mode ends; selecting Reserve does not replace the persisted advanced-model
default. API-key authentication does not satisfy the official activation gate.

## Cockpit's explicitly approved manual-selection design

Unlike the official automatic fallback UI, Cockpit exposes a permanent
**Luna Reserve** choice (`gpt-reserve`) for manual selection. This intentional
product difference was approved by the user on 2026-09-05.

- API Service and managed catalogs list Reserve regardless of whether any account
  is currently eligible. Saved managed lists also gain the option; existing
  entries and the user's default remain unchanged. Explicit API model allowlists,
  exclusions and prefixes still apply.
- Listing and model admission are separate from account selection. A request
  keeps the literal `gpt-reserve` ID (apart from explicit user aliases). Only
  scoped OAuth accounts satisfying all three quota predicates in the example
  above may be selected. Ordinary accounts and eligible accounts outside the
  client's API key scope cannot be used as fallback. Missing quota data fails
  closed. Authentication, disablement and runtime cooldown checks still apply.
- No eligible account means a normal account-pool error; there is no automatic
  switch to Luna, Astra or another model. Server acceptance remains authoritative.
- If a dedicated model template is absent, capabilities derive from the existing
  Luna template. A supplied dedicated template takes precedence. The display name
  is "Luna Reserve" to distinguish it from ordinary Luna.
- No fixed compaction threshold is introduced. Explicit user context settings
  remain explicit, and disabling the visible-model override uses the existing
  removal path.
- Neither the official installation nor official feature gates are patched. A
  local API-key session is not made to impersonate ChatGPT authentication or to
  activate the official moon indicator or automatic quota fallback.

The bundled Luna metadata can differ from account-specific server metadata.
This implementation preserves the currently integrated Luna template; it does
not claim separate official Reserve limits or copy all fields from a new
official bundle into unrelated models.

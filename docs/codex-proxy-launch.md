# Windows Codex account proxy launch

Account switching keeps the existing CODEX_HOME, Windows package identity,
project/history projection, MCP and skills behavior.

- Independent and shared account proxies use the existing stable loopback ingress.
- Client settings remains separate from an explicit force-direct choice.
- The existing managed .env projection preserves per-account originals, unrelated
  edits, absent/empty files, and recoverable encrypted ownership metadata.

## Optional country check inside Cockpit

The account proxy dialog exposes an independent country-check setting, disabled
by default. An account without a proxy binding can also choose an expected country.
The setting is saved separately from the proxy binding.

When switching desktop accounts through Cockpit, check the expected HTTP exit
before routing changes or credential preparation, even if automatic launch is
disabled. Ordinary instance startup also checks before proceeding, then rechecks
just before starting the GUI. Retry repeats the check; skipping a launch-progress step never skips it.
The check is not applied to prepare-only CLI actions.

A credential-free GET to ChatGPT Cloudflare trace reads its loc country code.
A mismatch shows expected and observed ISO country codes and blocks launch.
Timeouts, non-success responses, invalid/ambiguous country data or settings changes
also block launch. No cookies, OAuth tokens, quota checks or model requests are sent.
A selected proxy failure never falls back to another route.

Bound proxies use the same stable ingress as the client. Unbound modes inspect
the launch environment and the existing .env projection without changing either,
and use the existing system-proxy resolver when applicable. Unsupported .env
interpolation is not guessed. Country checking does not certify every client
protocol or dynamic PAC/Chromium-specific routing rule.

## Desktop behavior

There is no desktop-startup integration, background launcher, shortcut editor or
startup hook. This version does not inspect, replace, migrate or restore desktop
shortcuts. Opening Codex directly is outside the Cockpit startup check.

## Visible checks and editable client proxy variables

Country checking is a separate launch-progress step. Failure remains attached to
that step, retry repeats it, and the launch action cannot skip it. The account
dialog keeps its vertical layout and uses the existing searchable picker with
localized names, English names and ISO codes.

The independent results area shows expected/observed country, route source,
timestamp, duration and safe failure reason. Records are scoped to the account
and current Cockpit session, bounded to twelve entries. Persistent application
logs record the outcome without proxy URLs, credentials or IP addresses. Saved
network settings invalidate previous evidence; manual checks never start Codex.

Client settings shows recognized proxy assignments in one editable text area.
Only HTTP_PROXY, HTTPS_PROXY, ALL_PROXY and NO_PROXY are accepted, including
case aliases. Recognition failure allows manual input, but storage failure does
not silently overwrite saved settings. The encrypted per-account preference is
staged outside CODEX_HOME and applied at the next stopped-profile launch using
client settings. The existing ownership journal restores original proxy entries
on account changes and preserves unrelated .env contents. Independent/shared
proxy modes and force-direct continue using their existing behavior.

Client ENV controls appear only when using the original client settings on Windows.
Country-check history is a separate expandable section using the existing proxy styles.

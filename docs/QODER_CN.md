# Qoder CN platform

Qoder and Qoder CN are independent platforms, following the existing CodeBuddy / CodeBuddy CN
convention. There is no region selector inside the account dialog.

| | Qoder | Qoder CN |
|---|---|---|
| Platform ID | `qoder` | `qoder_cn` |
| Page | `qoder` | `qoder-cn` |
| Account store | `qoder_accounts/` | `qoder_cn_accounts/` |
| Typed account index | `QoderAccountIndex` | `QoderCnAccountIndex` |
| Device API | `openapi.qoder.sh` | `openapi.qoder.com.cn` |
| Native desktop profile | `com.qoder.app.stable` | `com.qodercn.app.stable` |
| CLI profile | `.qoder` | `.qoder-cn` |

Frontend stores, current-account references, groups, persisted filters, automatic refresh settings
and account transfers use the corresponding platform ID. Switching the global IDE and dedicated
deployments is unchanged.

## Credential markers

Every stored session carries a `login_source` marker, and the store only accepts the marker that
belongs to its own channel:

| Marker | Meaning |
|---|---|
| absent / `qoder` | global Qoder session |
| `qoder_cn` | public Qoder CN session (this platform) |
| `qodercn` | retired CN IDE; a file inside the CN store stamped with it is upgraded to `qoder_cn` on load |
| `dedicated_qodercn` | dedicated CN deployment, stays in its own IDE platform |

A global session cannot be imported into or refreshed by the CN platform, and CN requests never
reuse the global IDE's machine token. Each realm reads the machine cache
(`SharedClientCache/cache/machine_token.json`) from its own client tree — the global IDE's user
data directory for `qoder`, the CN desktop (`com.qodercn.app.stable`) or CLI (`.qoder-cn`) roots for
`qoder_cn`. A CN client that has never written such a cache contributes no machine headers instead
of borrowing the other realm's.

The same asymmetry holds for the stores: `import_qoder_from_json` rejects a payload stamped
`qoder_cn` (the CN page has its own import), and `import_qoder_cn_from_json` rejects everything else.

## Switching

Qoder CN supports device authorization, JSON and native import, quota refresh, and switching the
default native desktop or CLI client.

On Windows, a CN desktop switch resolves the launch path (configured → auto-detected → running
process), closes the running client, backs up the existing profile, writes atomically, verifies the
result, restores the backup if verification fails, and relaunches the client. Set a custom launch
path in Settings when the client is installed outside the usual locations; the detected path is
written back so the probe runs once.

Every entry point — the CN page, the dashboard, the floating card, automation and the native menu —
runs that same sequence. Writing credentials under a live client is refused by the adapter
(`QODER_CLIENT_RUNNING`), and a client that cannot be closed is reported instead of being started a
second time. If the client fails to come back the switch still counts as done: the credentials are
already in place, so the error code (`QODER_CN_RELAUNCH_FAILED`) tells the UI to refresh.

The same descriptor-driven adapters serve the global client, where the named targets are the Qoder
desktop app (`com.qoder.app.stable`) and the Qoder CLI (`.qoder`); the IDE target keeps using the
`state.vscdb` path. A global switch never reopens the client — close it first, then start it again
once the write is reported. Desktop writes also drop the per-account overlays
(`auth-profile-overlays.v1.dat`, `channel-activation.v1.json`), which describe the account that was
just replaced, and restore them if the write has to be rolled back.

The CLI target writes credentials only — reopen the terminal afterwards.

The automation protocol exposes the CN realm as the platform id `qoder_cn`, alongside the global
`qoder`, so scripts can list, look up, refresh and switch CN accounts through the same operations
the desktop host uses.

## Settings

- Launch path for the Qoder CN desktop client (empty means auto-detect).
- Auto refresh interval, current-account refresh and per-account overrides.
- Quota alerts: after a refresh, the current account's remaining credits are compared with the
  configured threshold, an alternative account is suggested, and a quick switch is offered.

## Limitations

- The desktop credential adapter is Windows-only for both realms; on macOS and Linux only the CLI
  target is available.
- Multiple instances of the CN desktop client are not supported.

## Attribution

Native profile and device-login conventions are adapted from
[WorkBuddy Switch Next](https://github.com/coderDJing/workbuddy-switch-next), licensed under MIT.
The license text ships in `resources/licenses/WorkBuddy-Switch-Next.txt` and in the application
bundle under `licenses/`.

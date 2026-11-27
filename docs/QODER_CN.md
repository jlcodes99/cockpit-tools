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
| Default macOS IDE profile | `Qoder/User/globalStorage/state.vscdb` | `QoderCN/User/globalStorage/state.vscdb` |
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
| `qodercn` | Qoder CN IDE native login marker; a file inside the Cockpit CN store stamped with it is upgraded to `qoder_cn` on load |
| `dedicated_qodercn` | dedicated CN deployment, stays in its own IDE platform |

A global session cannot be imported into or refreshed by the CN platform, and CN requests never
reuse the global IDE's machine token.

## Switching

Qoder CN supports device authorization, JSON import, local import from Desktop or CLI, and quota
refresh. In the account overview, the switch button opens a client picker with the following targets.

| Client | macOS | Windows | Linux |
|---|---|---|---|
| Qoder CN Desktop | Yes | Yes | |
| Qoder CN IDE | Yes | | |
| Qoder CN CLI | Yes | Yes | Yes |

**Switch and launch** closes the selected app if it is running, updates its sign-in, and opens it.
For CLI, quit any running Qoder CN CLI session first. Cockpit then opens a new terminal
and runs `qoderclicn` in the chosen directory. On macOS, open a fresh Qoder CN IDE installation once
to initialize its profile before switching.

Desktop and CLI switches back up the existing profile, write it atomically, and verify the result.
The IDE writes and verifies its credentials in a SQLite transaction. On macOS the desktop and IDE
use their own Electron Keychain entries and are matched by application bundle identifier.

The same descriptor-driven adapters serve the global client, where the named targets are the Qoder
desktop app (`com.qoder.app.stable`) and the Qoder CLI (`.qoder`); the IDE target keeps using the
`state.vscdb` path. On macOS, the international desktop app supports **Switch and launch**.
Its launch path and Electron Keychain entry are independent of the IDE and
the CN desktop app. Desktop writes also drop the per-account overlays
(`auth-profile-overlays.v1.dat`, `channel-activation.v1.json`), which describe the account that was
just replaced, and restore them if the write has to be rolled back.

The automation protocol exposes the CN realm as the platform id `qoder_cn`, alongside the global
`qoder`, so scripts can list, look up, refresh and switch CN accounts through the same operations
the desktop host uses. Client selection is available in the account overview.

## Settings

- Launch paths for the supported Qoder CN clients (empty means auto-detect).
- Auto refresh interval, current-account refresh and per-account overrides.
- Quota alerts: after a refresh, the current account's remaining credits are compared with the
  configured threshold, an alternative account is suggested, and a quick switch is offered.

## Limitations

- Qoder CN switching targets the default client profiles.

## Attribution

Native profile and device-login conventions are adapted from
[WorkBuddy Switch Next](https://github.com/coderDJing/workbuddy-switch-next), licensed under MIT.
The license text ships in `resources/licenses/WorkBuddy-Switch-Next.txt` and in the application
bundle under `licenses/`.

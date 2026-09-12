# Antigravity CLI Account Switch & Switch-and-Run Design Note

## 1. agy Version Compatibility Matrix

| Runtime Component | Inspected Environment Value | Notes |
|---|---|---|
| agy Executable Path | `~/.local/bin/agy` | Resolved via `which agy` / standard user PATH |
| agy Version | `1.1.27` | Verified via `agy --version` |
| Platform | macOS (Darwin arm64) | Tested on Apple Silicon |
| Target Backend | macOS Keychain | Service: `gemini`, Account: `antigravity` |
| Payload Format | `go-keyring-base64:<base64-json>` | `zalando/go-keyring` encoding wrapping standard OAuth payload |
| Cockpit Writer Compatibility | 100% Compatible | Fresh `agy` immediately picks up switched account without browser login |

## 2. Credential Backend Matrix

- **macOS**: Keychain via `security` CLI / Keychain API. Key: service `gemini`, account `antigravity`. Value formatted as `go-keyring-base64:<base64(json)>`.
- **Windows**: Windows Credential Manager via `advapi32`. Target: `gemini:antigravity`, User: `antigravity`. Value: JSON payload.
- **Linux**: Secret Service via `secret-tool`. Service: `gemini`, Username: `antigravity`. Value: JSON payload.

## 3. Transaction / Rollback Contract

### Switch Account (`switch_account_transaction`)

```text
Load Selected Account
       ↓
Prepare & Refresh Token (fail -> abort, return TOKEN_REFRESH_FAILED)
       ↓
Snapshot Current CLI Credential
       ↓
Write Selected Credential to Native Credential Store (fail -> rollback snapshot, return CREDENTIAL_WRITE_FAILED)
       ↓
Read-back / Local Verify Account Email (fail -> rollback snapshot, return CREDENTIAL_VERIFY_FAILED)
       ↓
Commit antigravity_cli Current Account Binding
       ↓
Return Switched Account
```

### Switch-and-Run (`run_antigravity_cli`)

```text
Execute Switch Transaction
       ↓
Switch Failed?
       ├─ Yes -> Stop immediately, do NOT launch agy, return switch error.
       └─ No  -> Credential committed. Proceed to launch.
                 Resolve agy executable
                 Spawn fresh agy process
                 Launch Failed?
                     ├─ Yes -> DO NOT rollback credential! Keep switch committed.
                     │         Return CLI_LAUNCH_FAILED error.
                     └─ No  -> Success (PID returned).
```

## 4. Known Unsupported Cases

- Hot-reloading active external `agy` sessions in other terminal tabs (guarantee is strictly for next / fresh launches).
- Automatic round-robin / quota-based auto-switching across CLI invocations.

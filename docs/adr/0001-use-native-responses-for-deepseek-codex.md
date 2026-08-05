# Use native Responses for the official DeepSeek Codex preset

The official DeepSeek preset speaks DeepSeek's native Responses API upstream and migrates existing DeepSeek provider and API-key account records to that protocol (`wire_api = "responses"`, catalog `deepseek-v4-flash` + `deepseek-v4-pro`), because the vendor now documents native Codex support and the Chat Completions bridge is no longer needed for these accounts.

Requests continue to flow through the local provider gateway on a loopback base URL — the gateway simply relays to DeepSeek using the Responses protocol instead of translating from Chat Completions. No direct external provider is written into `config.toml`, so Codex's sandbox configuration is left untouched. Users who still need the legacy Chat Completions protocol must create a custom provider.

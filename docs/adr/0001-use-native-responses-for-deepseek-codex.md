# Use native Responses for the official DeepSeek Codex preset

The official DeepSeek preset always uses DeepSeek's native Responses API and migrates existing DeepSeek provider and API-key account records to that protocol, because the vendor now documents native Codex support and the local Chat Completions bridge is unnecessary. Users who still need the legacy protocol must create a custom provider; the official preset keeps `deepseek-v4-flash` as the default while also publishing `deepseek-v4-pro` from DeepSeek's documented model catalog.

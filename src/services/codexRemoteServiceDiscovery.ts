import { invoke } from '@tauri-apps/api/core';
import type { CodexAccount } from '../types/codex';
import type { CodexRemoteConnection } from '../types/codexRemoteService';
import { CODEX_API_KEY_USAGE_REFRESHED_EVENT, readCodexApiKeyUsageCache, writeCodexApiKeyUsageCache } from './codexApiKeyUsageRefreshService';
import { queryModelProviderUsage } from './modelProviderUsageService';
import { codexRemoteBinding } from '../utils/codexRemoteBinding';

const pending = new Set<string>();
const checked = new Map<string, number>();
const latest = new Map<string, string>();
const queue: CodexAccount[] = [];
let active = 0;

/** Returns immediately; only two discovery requests run concurrently. */
export function discoverCodexRemoteServices(accounts: CodexAccount[]): void {
  for (const account of accounts) {
    if (account.auth_mode !== 'apikey' || !account.api_base_url || !account.openai_api_key) continue;
    const signature = JSON.stringify([account.id, account.api_base_url, account.openai_api_key]);
    latest.set(account.id, signature);
    if (pending.has(signature) || Date.now() - (checked.get(signature) ?? 0) < 300_000) continue;
    pending.add(signature);
    queue.push(account);
  }
  drain();
}

function drain(): void {
  while (active < 2 && queue.length) {
    const account = queue.shift()!;
    const signature = JSON.stringify([account.id, account.api_base_url, account.openai_api_key]);
    active++;
    void (async () => {
      try {
        const binding = await codexRemoteBinding(account.api_base_url!, account.openai_api_key!);
        if (latest.get(account.id) !== signature) return;
        const previous = readCodexApiKeyUsageCache();
        if (previous[account.id]?.summary?.cockpit && previous[account.id].summary?.sourceBinding !== binding) {
          delete previous[account.id];
          writeCodexApiKeyUsageCache(previous);
          window.dispatchEvent(new CustomEvent(CODEX_API_KEY_USAGE_REFRESHED_EVENT));
        }
        const remote = await invoke<CodexRemoteConnection | null>('codex_discover_remote_service', {
          baseUrl: account.api_base_url, apiKey: account.openai_api_key,
        });
        if (!remote) return;
        const summary = await queryModelProviderUsage({ baseUrl: account.api_base_url!, apiKey: account.openai_api_key! });
        if (latest.get(account.id) !== signature) return;
        const cache = readCodexApiKeyUsageCache();
        cache[account.id] = { loading: false, summary, updatedAt: Date.now() };
        writeCodexApiKeyUsageCache(cache);
        window.dispatchEvent(new CustomEvent(CODEX_API_KEY_USAGE_REFRESHED_EVENT));
      } catch {
        // Existing account data and last successful quota remain available offline.
      } finally {
        checked.set(signature, Date.now()); pending.delete(signature); active--; drain();
      }
    })();
  }
}

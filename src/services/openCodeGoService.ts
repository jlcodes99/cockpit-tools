import { invoke } from '@tauri-apps/api/core';
import type {
  OpenCodeGoConnection,
  OpenCodeGoQuotaQueryResult,
  OpenCodeGoQuotaSnapshot,
  OpenCodeGoQuotaWindowSnapshot,
} from '../types/openCodeGo';

export type { OpenCodeGoConnection as OpenCodeGoConnectionSummary } from '../types/openCodeGo';

export type OpenCodeGoUsageErrorKind =
  | 'authentication'
  | 'rate_limit'
  | 'network'
  | 'unavailable';

type UnknownRecord = Record<string, unknown>;

function record(value: unknown): UnknownRecord {
  return value !== null && typeof value === 'object' ? (value as UnknownRecord) : {};
}

function finiteNumber(value: unknown, field: string): number {
  const number = typeof value === 'number' ? value : Number.NaN;
  if (!Number.isFinite(number)) throw new Error(`OPENCODE_GO_INVALID_${field}`);
  return number;
}

function optionalFiniteNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function normalizeWindow(value: unknown): OpenCodeGoQuotaWindowSnapshot {
  const raw = record(value);
  const usedPercent = optionalFiniteNumber(raw.usedPercent);
  const remainingPercent = optionalFiniteNumber(raw.remainingPercent);
  const resetsAt = optionalFiniteNumber(raw.resetsAt);
  const status =
    raw.status === 'available' || raw.status === 'error' || raw.status === 'unavailable'
      ? raw.status
      : usedPercent != null || remainingPercent != null || resetsAt != null
        ? 'available'
        : 'unavailable';
  if (
    (usedPercent != null && (usedPercent < 0 || usedPercent > 100)) ||
    (remainingPercent != null && (remainingPercent < 0 || remainingPercent > 100)) ||
    (resetsAt != null && resetsAt <= 0)
  ) {
    throw new Error('OPENCODE_GO_INVALID_QUOTA_WINDOW');
  }
  const error = typeof raw.error === 'string' && raw.error.trim()
    ? raw.error.trim()
    : undefined;
  return { status, usedPercent, remainingPercent, resetsAt, ...(error ? { error } : {}) };
}

/** Validates only the sanitized quota shape exposed by the Tauri command. */
export function normalizeOpenCodeGoQuotaSnapshot(value: unknown): OpenCodeGoQuotaSnapshot {
  const raw = record(value);
  const status =
    raw.status === 'exhausted' || raw.status === 'partial' || raw.status === 'unavailable'
      ? raw.status
      : 'available';
  return {
    rolling: normalizeWindow(raw.rolling),
    weekly: normalizeWindow(raw.weekly),
    monthly: normalizeWindow(raw.monthly),
    status,
    queriedAt: finiteNumber(raw.queriedAt, 'QUERIED_AT'),
  };
}

/** Converts backend failure codes to presentation-safe classifications. */
export function toOpenCodeGoCommandError(error: unknown): OpenCodeGoUsageErrorKind {
  const message = String(error).toUpperCase();
  if (message.includes('AUTHENTICATION')) return 'authentication';
  if (message.includes('RATE_LIMIT')) return 'rate_limit';
  if (message.includes('NETWORK')) return 'network';
  return 'unavailable';
}

/** Normalizes the public, key-free connection contract at the Tauri boundary. */
export function normalizeOpenCodeGoConnection(value: unknown): OpenCodeGoConnection {
  const raw = record(value);
  const quotaErrorRaw = record(raw.quotaError);
  return {
    id: String(raw.id ?? '').trim(),
    name: String(raw.name ?? '').trim(),
    keyHint: String(raw.keyHint ?? '').trim(),
    ...(typeof raw.emailHint === 'string' && raw.emailHint.trim()
      ? { emailHint: raw.emailHint.trim() }
      : {}),
    createdAt: finiteNumber(raw.createdAt, 'CREATED_AT'),
    updatedAt: finiteNumber(raw.updatedAt, 'UPDATED_AT'),
    enabled: raw.enabled !== false,
    provider: raw.provider === 'zen' ? 'zen' : 'go',
    ...(raw.quota === undefined ? {} : { quota: normalizeOpenCodeGoQuotaSnapshot(raw.quota) }),
    ...(raw.quotaError === undefined
      ? {}
      : {
          quotaError: {
            kind: toOpenCodeGoCommandError(quotaErrorRaw.kind),
            occurredAt: finiteNumber(quotaErrorRaw.occurredAt, 'ERROR_OCCURRED_AT'),
          },
        }),
  };
}

/**
 * First-class frontend boundary for OpenCode Go connection ownership.
 * Consumers intentionally cannot read API-key material after creation/update.
 */
export interface OpenCodeGoConnectionService {
  listConnections(): Promise<OpenCodeGoConnection[]>;
  exportConnections(connectionIds: string[]): Promise<string>;
  importConnections(encryptedStore: string): Promise<OpenCodeGoConnection[]>;
  createConnection(input: { name: string; apiKey: string; email?: string; provider?: 'go' | 'zen' }): Promise<OpenCodeGoConnection>;
  updateConnection(
    connectionId: string,
    patch: { name?: string; apiKey?: string; email?: string },
  ): Promise<OpenCodeGoConnection>;
  setConnectionEnabled(connectionId: string, enabled: boolean): Promise<OpenCodeGoConnection>;
  deleteConnection(connectionId: string): Promise<void>;
  testConnection(connectionId: string): Promise<void>;
  queryQuota(connectionId: string): Promise<OpenCodeGoQuotaQueryResult>;
  queryAllQuotas(): Promise<OpenCodeGoConnection[]>;
}

export const openCodeGoService: OpenCodeGoConnectionService = {
  async listConnections() {
    const response = await invoke<unknown[]>('list_opencode_go_connections');
    return response.map(normalizeOpenCodeGoConnection);
  },

  async exportConnections(connectionIds) {
    return invoke<string>('export_opencode_go_connections', { connectionIds });
  },

  async importConnections(encryptedStore) {
    const response = await invoke<unknown[]>('import_opencode_go_connections', { encryptedStore });
    return response.map(normalizeOpenCodeGoConnection);
  },

  async createConnection({ name, apiKey, email, provider = 'go' }) {
    return normalizeOpenCodeGoConnection(await invoke('create_opencode_go_connection', {
      name: name.trim(),
      apiKey: apiKey.trim(),
      ...(email !== undefined ? { email: email.trim() } : {}),
      provider,
    }));
  },

  async updateConnection(connectionId, patch) {
    return normalizeOpenCodeGoConnection(await invoke('update_opencode_go_connection', {
      connectionId,
      name: patch.name?.trim(),
      apiKey: patch.apiKey?.trim(),
      ...(patch.email !== undefined ? { email: patch.email.trim() } : {}),
    }));
  },

  async setConnectionEnabled(connectionId, enabled) {
    return normalizeOpenCodeGoConnection(await invoke('set_opencode_go_connection_enabled', {
      connectionId,
      enabled,
    }));
  },

  async deleteConnection(connectionId) {
    await invoke<void>('delete_opencode_go_connection', { connectionId });
  },

  async testConnection(connectionId) {
    await invoke<void>('test_opencode_connection', { connectionId });
  },

  async queryQuota(connectionId) {
    const raw = record(await invoke('query_opencode_go_quota', { connectionId }));
    return {
      connection: normalizeOpenCodeGoConnection(raw.connection),
      quota: normalizeOpenCodeGoQuotaSnapshot(raw.quota),
    };
  },

  async queryAllQuotas() {
    const raw = record(await invoke('query_all_opencode_go_quotas'));
    const connections = Array.isArray(raw.connections) ? raw.connections : [];
    return connections.map(normalizeOpenCodeGoConnection);
  },
};

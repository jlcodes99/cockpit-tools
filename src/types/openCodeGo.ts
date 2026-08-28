export type OpenCodeGoQuotaWindowStatus = 'available' | 'unavailable' | 'error';

export interface OpenCodeGoQuotaWindowSnapshot {
  status?: OpenCodeGoQuotaWindowStatus;
  usedPercent: number | null;
  remainingPercent: number | null;
  resetsAt: number | null;
  error?: string;
}

export interface OpenCodeGoQuotaSnapshot {
  rolling: OpenCodeGoQuotaWindowSnapshot;
  weekly: OpenCodeGoQuotaWindowSnapshot;
  monthly: OpenCodeGoQuotaWindowSnapshot;
  /** All windows available, a zero-remaining window, or a partial/unavailable response. */
  status: 'available' | 'exhausted' | 'partial' | 'unavailable';
  queriedAt: number;
}

export interface OpenCodeGoQuotaError {
  kind: string;
  occurredAt: number;
}

/** Sanitized connection metadata returned by the backend. */
export interface OpenCodeGoConnection {
  id: string;
  name: string;
  keyHint: string;
  /** Masked local owner identity; raw email remains in encrypted storage. */
  emailHint?: string;
  createdAt: number;
  updatedAt: number;
  /** Missing values are legacy Go connections and therefore enabled. */
  enabled?: boolean;
  provider?: 'go' | 'zen';
  quota?: OpenCodeGoQuotaSnapshot;
  quotaError?: OpenCodeGoQuotaError;
}

export interface OpenCodeGoQuotaQueryResult {
  connection: OpenCodeGoConnection;
  quota: OpenCodeGoQuotaSnapshot;
}

export interface OpenCodeGoQuotaBatchResult {
  connections: OpenCodeGoConnection[];
}

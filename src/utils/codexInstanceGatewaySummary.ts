import type { CodexInstanceGatewayView } from '../types/codexLocalAccess';

type GatewayHealth = Pick<CodexInstanceGatewayView, 'status' | 'lastError'>;

/** A stopped gateway is normal after an explicit stop; retained failures still count. */
export function summarizeCodexInstanceGateways(gateways: readonly GatewayHealth[]) {
  return {
    total: gateways.length,
    running: gateways.filter((gateway) => gateway.status === 'running').length,
    issues: gateways.filter((gateway) => gateway.status === 'unreachable'
      || gateway.status === 'portConflict' || Boolean(gateway.lastError?.trim())).length,
  };
}

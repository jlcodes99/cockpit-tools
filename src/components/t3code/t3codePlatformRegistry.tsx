import type { Page } from '../../types/navigation';
import type { PlatformId } from '../../types/platform';
import type {
  T3CodeBindableAccountView,
  T3CodeDriverKind,
  T3CodeProviderInstanceView,
  T3CodeProviderWorkspaceView,
} from '../../types/t3code';

export interface T3codePlatformSpec {
  driver: T3CodeDriverKind;
  label: string;
  platformId: PlatformId;
  navPage: Page;
  getAccounts: (workspace: T3CodeProviderWorkspaceView | null) => T3CodeBindableAccountView[];
}

export const T3CODE_PLATFORM_SPECS: readonly T3codePlatformSpec[] = [
  {
    driver: 'antigravity',
    label: 'Antigravity',
    platformId: 'antigravity',
    navPage: 'overview',
    getAccounts: (workspace) => workspace?.antigravityAccounts ?? [],
  },
  {
    driver: 'codex',
    label: 'Codex',
    platformId: 'codex',
    navPage: 'codex',
    getAccounts: (workspace) => workspace?.codexAccounts ?? [],
  },
  {
    driver: 'claudeAgent',
    label: 'Claude Code',
    platformId: 'claude_manager',
    navPage: 'claude-cli',
    getAccounts: (workspace) => workspace?.claudeAccounts ?? [],
  },
  {
    driver: 'workbuddy',
    label: 'WorkBuddy',
    platformId: 'workbuddy',
    navPage: 'workbuddy',
    getAccounts: (workspace) => workspace?.extraAccounts?.workbuddy ?? [],
  },
  {
    driver: 'workbuddy_ai',
    label: 'WorkBuddy AI',
    platformId: 'workbuddy_ai',
    navPage: 'workbuddy_ai',
    getAccounts: (workspace) => workspace?.extraAccounts?.workbuddy_ai ?? [],
  },
  {
    driver: 'codebuddy',
    label: 'CodeBuddy',
    platformId: 'codebuddy',
    navPage: 'codebuddy',
    getAccounts: (workspace) => workspace?.extraAccounts?.codebuddy ?? [],
  },
  {
    driver: 'codebuddy_cn',
    label: 'CodeBuddy CN',
    platformId: 'codebuddy_cn',
    navPage: 'codebuddy-cn',
    getAccounts: (workspace) => workspace?.extraAccounts?.codebuddy_cn ?? [],
  },
  {
    driver: 'grok',
    label: 'Grok',
    platformId: 'grok',
    navPage: 'grok',
    getAccounts: (workspace) => workspace?.extraAccounts?.grok ?? [],
  },
  {
    driver: 'minimaxcode',
    label: 'MiniMax Code',
    platformId: 'minimaxcode',
    navPage: 'minimaxcode',
    getAccounts: (workspace) => workspace?.extraAccounts?.minimaxcode ?? [],
  },
];

const PLATFORM_SPEC_MAP = new Map<T3CodeDriverKind, T3codePlatformSpec>(
  T3CODE_PLATFORM_SPECS.map((spec) => [spec.driver, spec]),
);

export function getOrderedT3codePlatforms(savedOrder: string | null): readonly T3codePlatformSpec[] {
  if (!savedOrder) return T3CODE_PLATFORM_SPECS;
  let parsed: unknown;
  try {
    parsed = JSON.parse(savedOrder);
  } catch {
    return T3CODE_PLATFORM_SPECS;
  }
  if (!Array.isArray(parsed)) return T3CODE_PLATFORM_SPECS;

  const remaining = new Map<string, T3codePlatformSpec>(PLATFORM_SPEC_MAP);
  const ordered: T3codePlatformSpec[] = [];
  for (const driver of parsed) {
    if (typeof driver !== 'string') continue;
    const spec = remaining.get(driver);
    if (!spec) continue;
    ordered.push(spec);
    remaining.delete(driver);
  }
  return [...ordered, ...remaining.values()];
}

export function getT3codePlatformSpec(driver: T3CodeDriverKind): T3codePlatformSpec {
  return PLATFORM_SPEC_MAP.get(driver) ?? T3CODE_PLATFORM_SPECS[0];
}

export function getAccountsForDriver(
  workspace: T3CodeProviderWorkspaceView | null,
  driver: T3CodeDriverKind,
): T3CodeBindableAccountView[] {
  return getT3codePlatformSpec(driver).getAccounts(workspace);
}

export function getDefaultProvider(
  workspace: T3CodeProviderWorkspaceView | null,
  driver: T3CodeDriverKind,
): T3CodeProviderInstanceView | undefined {
  const instances = workspace?.providerInstances ?? [];
  return instances.find((item) => item.driver === driver && (item.isBuiltinDefault || item.instanceId === driver));
}

export function getAvailableProviderAccounts(
  workspace: T3CodeProviderWorkspaceView | null,
  driver: T3CodeDriverKind,
): T3CodeBindableAccountView[] {
  const assigned = new Set((workspace?.providerInstances ?? [])
    .filter((instance) => instance.driver === driver)
    .map((instance) => instance.boundAccountId));
  return getAccountsForDriver(workspace, driver).filter((account) => !assigned.has(account.id));
}

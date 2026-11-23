import type { CodexCountryCheck } from '../services/codexProxyCountryService';
import type { CodexLaunchStepStatus } from './codexLaunchProgress';

interface CountryProgressState {
  accountId?: string;
  status: string;
  steps: { id: string; status: CodexLaunchStepStatus; details: Record<string, unknown> }[];
}
export function applyCountryProgress<T extends CountryProgressState>(state: T | null, value: CodexCountryCheck): T | null {
  const key = (id?: string) => (id || '__client__').replace(/^__provider_gateway__:/, '');
  if (!state || state.status !== 'running' || value.scope !== 'launch' || key(state.accountId) !== key(value.accountId)) return state;
  return { ...state, steps: state.steps.map(step => step.id === 'checkProxyCountry' ? {
    ...step,
    status: value.status === 'checking' ? 'running' : value.status === 'passed' ? 'completed' : 'error',
    details: { ...value, error: value.reason ?? undefined },
  } : step) };
}
export function countryProgressLines(details: Record<string, unknown>, t: (key: string, values?: Record<string, unknown>) => string): string[] {
  const lines = [t('codex.proxy.country.progressCountries', {
    expected: details.expectedCountry || t('codex.proxy.country.notRequired'),
    actual: details.actualCountry || t('codex.proxy.country.notDetected'),
  })];
  if (typeof details.elapsedMs === 'number') lines.push(t('codex.proxy.country.progressTime', { ms: details.elapsedMs }));
  return lines;
}

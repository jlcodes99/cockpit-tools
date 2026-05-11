import type { TFunction } from 'i18next';

const CODEX_LOCAL_ACCESS_RISK_NOTICE_DISMISSED_KEY =
  'agtools.codex.local_access.risk_notice.dismissed.v2';

export type CodexLocalAccessRiskNoticeAction = 'service' | 'switch';

export function isCodexLocalAccessRiskNoticeDismissed(): boolean {
  try {
    return localStorage.getItem(CODEX_LOCAL_ACCESS_RISK_NOTICE_DISMISSED_KEY) === '1';
  } catch {
    return false;
  }
}

export function setCodexLocalAccessRiskNoticeDismissed(value: boolean): void {
  try {
    if (value) {
      localStorage.setItem(CODEX_LOCAL_ACCESS_RISK_NOTICE_DISMISSED_KEY, '1');
      return;
    }
    localStorage.removeItem(CODEX_LOCAL_ACCESS_RISK_NOTICE_DISMISSED_KEY);
  } catch {
    // ignore storage write failures
  }
}

export function getCodexLocalAccessRiskNoticeConfirmLabel(
  action: CodexLocalAccessRiskNoticeAction,
  t: TFunction,
): string {
  if (action === 'switch') {
    return t('codex.localAccess.riskNotice.continueSwitch', "Continue Switch");
  }
  if (action === 'service') {
    return t('codex.localAccess.riskNotice.continueStart', "Continue");
  }
  return t('common.confirm', "Confirm");
}

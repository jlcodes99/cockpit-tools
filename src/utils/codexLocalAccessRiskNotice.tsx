import type { TFunction } from 'i18next';
import {
  isUserMemoryDismissed,
  markUserMemoryDismissed,
  USER_MEMORY_FLAGS,
} from './userMemory';

export type CodexLocalAccessRiskNoticeAction = 'service' | 'switch';

export function isCodexLocalAccessRiskNoticeDismissed(): boolean {
  return isUserMemoryDismissed(USER_MEMORY_FLAGS.riskNotice);
}

export function setCodexLocalAccessRiskNoticeDismissed(value: boolean): void {
  if (value) {
    void markUserMemoryDismissed(USER_MEMORY_FLAGS.riskNotice);
  }
}

export function getCodexLocalAccessRiskNoticeConfirmLabel(
  action: CodexLocalAccessRiskNoticeAction,
  t: TFunction,
): string {
  if (action === 'switch') {
    return t('codex.localAccess.riskNotice.continueSwitch', '继续切号');
  }
  if (action === 'service') {
    return t('codex.localAccess.riskNotice.continueStart', '继续启动');
  }
  return t('common.confirm', '确认');
}

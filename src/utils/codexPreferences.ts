const CODEX_SHOW_CODE_REVIEW_QUOTA_STORAGE_KEY = 'agtools.codex_show_code_review_quota';

export const CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT =
  'agtools:codex-code-review-quota-visibility-changed';

export function isCodexCodeReviewQuotaVisibleByDefault(): boolean {
  try {
    return localStorage.getItem(CODEX_SHOW_CODE_REVIEW_QUOTA_STORAGE_KEY) === '1';
  } catch {
    return false;
  }
}

export function persistCodexCodeReviewQuotaVisible(visible: boolean): void {
  try {
    localStorage.setItem(CODEX_SHOW_CODE_REVIEW_QUOTA_STORAGE_KEY, visible ? '1' : '0');
    window.dispatchEvent(
      new CustomEvent(CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT, { detail: visible }),
    );
  } catch {
    // ignore localStorage write failures
  }
}

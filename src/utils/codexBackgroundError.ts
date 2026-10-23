import type { TFunction } from 'i18next';
import { chatGptPlanErrorKey } from '../services/chatgptPlanService';

/** Preserve ordinary diagnostics; official credential errors use shared translations. */
export function codexBackgroundError(error: unknown, t: TFunction): string {
  const message = error instanceof Error ? error.message : String(error);
  return /^CHATGPT_PLAN_[A-Z_]+$/.test(message) ? t(chatGptPlanErrorKey(message)) : message;
}

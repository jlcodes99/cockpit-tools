import { getCodexResetCredits } from './codexService';
import { createResetCreditLoader } from '../utils/codexResetCreditLoader';

export const codexResetCreditDetails = createResetCreditLoader(getCodexResetCredits, {
  storage: localStorage,
});

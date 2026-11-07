import type { TFunction } from 'i18next';
import { formatOutputSpeed, outputTokensPerSecond, outputSpeedUnavailableReason, type OutputSpeedSample, type OutputSpeedUnavailableReason } from '../../utils/codexOutputSpeed';
import './CodexOutputSpeed.css';

const unavailableReasonKeys: Record<OutputSpeedUnavailableReason, string> = {
  canceled: 'common.outputSpeed.unavailableReasons.canceled',
  failed: 'common.outputSpeed.unavailableReasons.failed',
  missingUsage: 'common.outputSpeed.unavailableReasons.missingUsage',
  shortOutput: 'common.outputSpeed.unavailableReasons.shortOutput',
  missingTiming: 'common.outputSpeed.unavailableReasons.missingTiming',
  shortWindow: 'common.outputSpeed.unavailableReasons.shortWindow',
};

export function CodexOutputSpeed({ event, t, hideLabel = false }: { event: OutputSpeedSample; t: TFunction; hideLabel?: boolean }) {
  const value = formatOutputSpeed(outputTokensPerSecond(event));
  const reason = outputSpeedUnavailableReason(event);
  const hint = value != null ? t('common.outputSpeed.hint')
    : reason ? t(unavailableReasonKeys[reason]) : t('common.outputSpeed.unavailableHint');
  return <span className="codex-output-speed" title={hint}>
    {hideLabel ? '' : t('common.outputSpeed.label')}{hideLabel ? '' : ' '}{value == null ? '—' : t('common.outputSpeed.value', { value })}
  </span>;
}

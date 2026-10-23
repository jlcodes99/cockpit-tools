import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import type { CodexPelicanItemStatus } from '../../../types/codexPelican';
import { gradePelicanReply } from './pelicanScoring';
import { scoredPelicanBenchmark } from './pelicanBenchmarks';

export function PelicanScore({ prompt, reply, status }: { prompt: string; reply?: string; status: CodexPelicanItemStatus }) {
  const { t } = useTranslation();
  const score = useMemo(() => status === 'completed' && reply !== undefined ? gradePelicanReply(prompt, reply) : null, [prompt, reply, status]);
  // Transport failures, cancellation and interrupted replies never become zero scores.
  if (status !== 'completed') return null;
  if (!scoredPelicanBenchmark(prompt)) return <p className="pelican-score-note">{t('pelican.grading.manual')}</p>;
  if (!score) return <p className="pelican-score-note">{t('pelican.grading.waiting')}</p>;
  if (score.status === 'tooLong') return <p className="pelican-score-note">{t('pelican.grading.tooLong')}</p>;
  const answerKey = { passed: 'answerPassed', partial: 'answerPartial', failed: 'answerFailed', unavailable: 'answerUnavailable' };
  return <details className={`pelican-score pelican-score-${score.status}`}>
    <summary>
      <span>{t(score.format === 'passed' ? 'pelican.grading.formatPassed' : 'pelican.grading.formatFailed')}</span>
      {score.answer && <span>{t(`pelican.grading.${answerKey[score.answer]}`)}{score.answer !== 'unavailable' && <> · {score.passed}/{score.total}</>}</span>}
    </summary>
    <p className="pelican-score-note">{t(score.answer === null ? 'pelican.grading.formatOnly' : 'pelican.grading.answerScope')}</p>
    <ul>{score.checks.map((check, index) => <li key={index}>
      <span>{t(check.passed ? 'pelican.grading.checkPassed' : 'pelican.grading.checkFailed')} · {t(`pelican.gradeRules.${check.kind}`, { count: check.count })}{check.field ? ` · ${check.field}` : ''}</span>
      {check.expected !== undefined && <span>{t('pelican.grading.expected', { value: check.expected })}</span>}
      {check.actual !== undefined && <span>{t('pelican.grading.actual', { value: check.actual })}</span>}
    </li>)}</ul>
  </details>;
}

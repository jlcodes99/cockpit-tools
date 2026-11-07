import { useTranslation } from 'react-i18next';
import { pelicanBenchmark, scoredPelicanBenchmark } from './pelicanBenchmarks';
import notices from './benchmarks/benchmarkNotices.json';

export function PelicanReference({ prompt }: { prompt: string }) {
  const { t } = useTranslation();
  const benchmark = scoredPelicanBenchmark(prompt);
  if (!benchmark && pelicanBenchmark(prompt) !== 'candy') return null;
  return <details className="pelican-prompt-details pelican-reference" open>
    <summary>{t('pelican.reference')}</summary>
    {benchmark ? <>
      <p className="pelican-muted">{benchmark.suite === 'Cockpit' ? t('pelican.originalCase') : <a href={benchmark.sourceUrl} target="_blank" rel="noreferrer">{benchmark.suite} · {benchmark.sourceId}</a>} · {benchmark.release} · {benchmark.graderVersion}</p>
      <ul>{benchmark.rules.map((rule, index) => <li key={index}>
        {t(`pelican.gradeRules.${rule.kind}`, { count: 'count' in rule ? rule.count : undefined })}
        {'answer' in rule && <p>{t('pelican.grading.expected', { value: rule.answer })}</p>}
        {'answers' in rule && <p>{t('pelican.grading.expected', { value: rule.answers.join(', ') })}</p>}
        {'fields' in rule && <pre>{JSON.stringify(Object.fromEntries(rule.fields.map((field) => [field.name, field.value])), null, 2)}</pre>}
      </li>)}</ul>
      {benchmark.reference && <p className="pelican-muted">{benchmark.reference}</p>}
      <p className="pelican-muted">{t(benchmark.rules.every((rule) => ['json', 'noComma', 'bullets', 'placeholders'].includes(rule.kind)) ? 'pelican.grading.formatOnly' : 'pelican.grading.answerScope')}</p>
      {benchmark.suite !== 'Cockpit' && <details className="pelican-notices"><summary>{t('pelican.license')}</summary><pre>{notices[benchmark.suite]}</pre></details>}
    </> : <p className="pelican-muted">{t('pelican.candyReference')}</p>}
  </details>;
}

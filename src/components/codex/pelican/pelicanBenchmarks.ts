import { CODEX_PELICAN_DEFAULT_PROMPT } from '../../../types/codexPelican.ts';
import { SCORED_BENCHMARKS, SELECTABLE_SCORED_BENCHMARKS } from './benchmarks/benchmarkCases.ts';
import type { ScoredBenchmark } from './benchmarks/benchmarkTypes.ts';

// Fixed benchmark data: keep the same language and wording across accounts and UI locales.
// Reference answers are UI-only and must never be appended to the request.
export const CANDY_REASONING_PROMPT = `不要搜索资料，自己思考：

袋子里有三种口味的糖——苹果、桃子、西瓜。每种口味又有两种形状：圆形和五角星。摸糖的时候，形状靠手感就能分辨，但不能分辨口味。

不同口味和形状的数量如下：
形状 | 苹果 | 桃子 | 西瓜
圆形 | 7 | 9 | 8
五角星 | 7 | 6 | 4

问题是：最少取多少颗，能保证手里同时有一组「圆形苹果 + 五角星桃子」或者「五角星苹果 + 圆形桃子」？

允许按形状选择拿取，每颗取出后不放回，不能根据口味挑选，也不能在取出后获知口味。请给出最少数量、保证成功的取法，以及更少数量不能保证成功的理由。用纯文本回答，不要生成 HTML。`;

export const PELICAN_BENCHMARKS: Readonly<Record<string, string>> = {
  animation: CODEX_PELICAN_DEFAULT_PROMPT,
  candy: CANDY_REASONING_PROMPT,
  ...Object.fromEntries(SCORED_BENCHMARKS.map((entry) => [entry.id, entry.prompt])),
  custom: '',
};

// Selection and lookup are separate: retiring a case never rewrites saved prompts.
export const PELICAN_BENCHMARK_IDS = ['animation', 'candy', ...SELECTABLE_SCORED_BENCHMARKS.map((entry) => entry.id), 'custom'];

export type PelicanBenchmark = keyof typeof PELICAN_BENCHMARKS;

export function pelicanBenchmark(prompt: string): PelicanBenchmark {
  return Object.keys(PELICAN_BENCHMARKS).find((id) => id !== 'custom' && PELICAN_BENCHMARKS[id] === prompt) ?? 'custom';
}

export function scoredPelicanBenchmark(prompt: string): ScoredBenchmark | undefined {
  return SCORED_BENCHMARKS.find((entry) => entry.prompt === prompt);
}

export function pelicanBenchmarkLabel(id: string, t: (key: string, options?: { number?: number }) => string): string {
  const benchmark = SCORED_BENCHMARKS.find((entry) => entry.id === id) as ScoredBenchmark | undefined;
  return benchmark ? `${benchmark.suite} · ${t(benchmark.labelKey, { number: benchmark.labelNumber })}` : t(`pelican.benchmarks.${id}`);
}

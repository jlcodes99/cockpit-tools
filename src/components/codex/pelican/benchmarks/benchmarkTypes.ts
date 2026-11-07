export type JsonValue = null | boolean | number | string | readonly JsonValue[] | { readonly [key: string]: JsonValue };

export type BenchmarkRule =
  | { kind: 'json' | 'noComma' }
  | { kind: 'bullets' | 'placeholders'; count: number }
  | { kind: 'choice' | 'boldInteger'; answer: string }
  | { kind: 'solution'; answers: readonly string[] }
  | { kind: 'jsonFields'; fields: readonly { name: string; value: JsonValue }[] };

export interface ScoredBenchmark {
  id: string;
  suite: 'IFEval' | 'SimpleBench' | 'LiveBench' | 'Cockpit';
  labelKey: string;
  labelNumber?: number;
  sourceId: string;
  sourceUrl: string;
  sourceRevision: string;
  release: string;
  // Frozen local grader contract, independent of future upstream leaderboard changes.
  graderVersion: 'cockpit-strict-v1' | 'cockpit-exact-v2';
  reference?: string;
  prompt: string;
  rules: readonly BenchmarkRule[];
}

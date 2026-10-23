import { scoredPelicanBenchmark } from './pelicanBenchmarks.ts';
import { exactJsonEqual, parseExactJson } from './benchmarks/strictJson.ts';
import type { BenchmarkRule, JsonValue } from './benchmarks/benchmarkTypes.ts';

export interface PelicanScoreCheck {
  kind: BenchmarkRule['kind'];
  passed: boolean;
  dimension: 'format' | 'answer';
  field?: string;
  count?: number;
  expected?: string;
  actual?: string;
}

export interface PelicanScoreResult {
  version: 'cockpit-strict-v1' | 'cockpit-exact-v2';
  format: 'passed' | 'failed' | null;
  answer: 'passed' | 'partial' | 'failed' | 'unavailable' | null;
  status: 'passed' | 'failed' | 'partial' | 'invalid' | 'tooLong';
  passed: number;
  total: number;
  checks: PelicanScoreCheck[];
}

// Synchronous local grading must remain bounded on the UI thread. Never grade a
// truncated prefix: oversized replies are explicitly ungraded, not failed.
export const PELICAN_GRADING_MAX_CHARS = 64 * 1024;

function evaluate(rule: BenchmarkRule, reply: string): { valid: boolean; checks: PelicanScoreCheck[]; formatPassed?: boolean; answerValid?: boolean } {
  const dimension = ['json', 'noComma', 'bullets', 'placeholders'].includes(rule.kind) ? 'format' : 'answer';
  let passed = false;
  let actual: string | undefined;
  let expected: string | undefined;
  let valid = true;
  switch (rule.kind) {
    case 'jsonFields': {
      let parsed;
      try { parsed = parseExactJson(reply.trim()); } catch { /* Keep ambiguous or malformed answers ungraded. */ }
      const object = parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as Record<string, JsonValue> : undefined;
      const names = rule.fields.map((field) => field.name);
      const formatPassed = !!object && Object.keys(object).length === names.length && names.every((name) => Object.prototype.hasOwnProperty.call(object, name));
      return { valid: formatPassed, formatPassed, answerValid: !!object,
        checks: rule.fields.map((field) => ({ kind: rule.kind, dimension: 'answer', field: field.name,
          expected: JSON.stringify(field.value), actual: object && Object.prototype.hasOwnProperty.call(object, field.name) ? JSON.stringify(object[field.name]) : undefined,
          passed: !!object && exactJsonEqual(object[field.name], field.value) })) };
    }
    case 'json': {
      const json = reply.trim().replace(/^```(?:json)?\s*\n?([\s\S]*?)\n?```$/i, '$1').trim();
      try { JSON.parse(json); passed = true; } catch { /* Invalid JSON is a failed format check. */ }
      break;
    }
    case 'noComma':
      passed = !reply.includes(',');
      break;
    case 'bullets':
      actual = String((reply.match(/^[ \t]*\*[^*\r\n][^\r\n]*$|^[ \t]*-[^\r\n]*$/gm) ?? []).length);
      expected = String(rule.count);
      passed = actual === expected;
      break;
    case 'placeholders': {
      // A regex starting at every unmatched '[' can take quadratic time on
      // adversarial replies. This scanner matches non-overlapping same-line pairs.
      let opened = false;
      let count = 0;
      for (const character of reply) {
        if (character === '\n' || character === '\r') opened = false;
        else if (character === '[') opened = true;
        else if (character === ']' && opened) { count++; opened = false; }
      }
      actual = String(count);
      expected = String(rule.count);
      passed = Number(actual) >= rule.count;
      break;
    }
    case 'choice': {
      const matches = [...reply.matchAll(/^Answer:[ \t]*([A-F])[ \t]*\r?$/gim)];
      valid = matches.length === 1 && reply.slice(matches[0].index! + matches[0][0].length).trim() === '';
      actual = valid ? matches[0][1].toUpperCase() : undefined;
      expected = rule.answer;
      passed = valid && actual === expected;
      break;
    }
    case 'boldInteger': {
      const match = /\*\*([0-9]{1,6})\*\*\s*[.!]?\s*$/.exec(reply);
      valid = !!match;
      actual = match ? String(Number(match[1])) : undefined;
      expected = rule.answer;
      passed = valid && actual === expected;
      break;
    }
    case 'solution': {
      // Require one unambiguous final solution. Never search reasoning text for
      // reference words or accept a substring (e.g. "not journalist").
      const start = reply.indexOf('<solution>');
      const end = reply.indexOf('</solution>');
      valid = start >= 0 && end > start && start === reply.lastIndexOf('<solution>')
        && end === reply.lastIndexOf('</solution>') && !reply.slice(end + '</solution>'.length).trim();
      const answers = valid ? reply.slice(start + '<solution>'.length, end).split(',').map((word) => word.trim()) : [];
      valid = valid && answers.length === rule.answers.length && answers.every(Boolean);
      const normalize = (word: string) => word.toLowerCase().replace(/-/g, ' ').replace(/\s+/g, ' ').trim();
      return { valid, checks: rule.answers.map((answer, index) => ({ kind: rule.kind, dimension,
        expected: answer, actual: valid ? answers[index] : undefined,
        passed: valid && normalize(answers[index]) === normalize(answer) })) };
    }
  }
  return { valid, checks: [{ kind: rule.kind, dimension, passed, expected, actual, ...('count' in rule ? { count: rule.count } : {}) }] };
}

export function gradePelicanReply(prompt: string, reply: string): PelicanScoreResult | null {
  const benchmark = scoredPelicanBenchmark(prompt);
  if (!benchmark) return null;
  const base = { version: benchmark.graderVersion, passed: 0, total: 0, checks: [], format: null, answer: null };
  if (reply.length > PELICAN_GRADING_MAX_CHARS) return { ...base, status: 'tooLong' };
  const evaluations = benchmark.rules.map((rule) => evaluate(rule, reply));
  const checks = evaluations.flatMap((entry) => entry.checks);
  // Upstream evaluators reject empty replies even for negative constraints.
  if (!reply.trim()) checks.forEach((check) => { check.passed = false; });
  const passed = checks.filter((check) => check.passed).length;
  const status = evaluations.some((entry) => !entry.valid) ? 'invalid'
    : passed === checks.length ? 'passed' : passed ? 'partial' : 'failed';
  const formatChecks = checks.filter((check) => check.dimension === 'format');
  const answerChecks = checks.filter((check) => check.dimension === 'answer');
  const answerValid = evaluations.every((entry) => entry.answerValid ?? entry.valid);
  const answerPassed = answerChecks.filter((check) => check.passed).length;
  return { version: benchmark.graderVersion, status, passed, total: checks.length, checks,
    format: reply.trim() && evaluations.every((entry) => entry.formatPassed ?? entry.valid) && formatChecks.every((check) => check.passed) ? 'passed' : 'failed',
    answer: !answerChecks.length ? null : !answerValid ? 'unavailable'
      : answerPassed === answerChecks.length ? 'passed' : answerPassed ? 'partial' : 'failed' };

}

// Original fixed benchmark fixtures. English below is benchmark data, not UI copy.
// Treat prompts, expected values and grading versions as immutable history contracts.
import type { ScoredBenchmark } from './benchmarkTypes.ts';

export interface LedgerRecord {
  eventId: string;
  project: string;
  period: string;
  invoice: string;
  revision: number;
  state: 'posted' | 'void';
  amountCents: number;
}

// Deliberately not sorted by revision: later rows do not necessarily supersede earlier rows.
const TARGET_UPDATES: readonly (readonly [string, string, number, LedgerRecord['state'], number])[] = [
  ['T01', 'A101', 1, 'posted', 12050], ['T03', 'A101', 3, 'void', 0],
  ['T05', 'A102', 2, 'posted', 8100], ['T06', 'A103', 1, 'posted', 30000],
  ['T09', 'A104', 1, 'posted', 1999], ['T12', 'A105', 2, 'void', 0],
  ['T15', 'A106', 3, 'posted', 9999], ['T16', 'A107', 1, 'posted', 2500],
  ['T18', 'A108', 2, 'posted', 7500], ['T07', 'A103', 2, 'void', 0],
  ['T02', 'A101', 2, 'posted', 12990], ['T04', 'A102', 1, 'posted', 8400],
  ['T08', 'A103', 3, 'posted', 29025], ['T10', 'A104', 2, 'posted', 2199],
  ['T11', 'A105', 1, 'posted', 45000], ['T13', 'A106', 1, 'posted', 10000],
  ['T14', 'A106', 2, 'posted', 10555], ['T17', 'A108', 1, 'posted', 7000],
];
const TARGET_EVENTS: readonly LedgerRecord[] = TARGET_UPDATES.map(([eventId, invoice, revision, state, amountCents]) => ({
  eventId, project: 'ORBIT', period: '2026-09', invoice, revision, state, amountCents,
}));

export function buildContextLedgerRecords(): readonly LedgerRecord[] {
  const records: LedgerRecord[] = [];
  for (let index = 0; index < 240; index++) {
    // Conflicting scopes reuse invoice IDs and have higher revisions and plausible amounts.
    records.push({
      eventId: `N${String(index + 1).padStart(4, '0')}`,
      project: index % 3 === 0 ? 'ORBIT' : index % 3 === 1 ? 'ORBIT-ARCHIVE' : 'LYRA',
      period: index % 3 === 0 ? '2026-08' : '2026-09',
      invoice: `A${101 + index % 8}`, revision: 10 + Math.floor(index / 8),
      state: index % 7 === 0 ? 'void' : 'posted',
      amountCents: index % 7 === 0 ? 0 : 1100 + (index * 733) % 48000,
    });
    if (index % 13 === 6) {
      const target = TARGET_EVENTS[Math.floor(index / 13)];
      if (target) records.push({ ...target });
    }
    // Repeated old deliveries near the end must neither add money nor override new versions.
    if (index === 199 || index === 219 || index === 239) {
      records.push({ ...TARGET_EVENTS[index === 199 ? 0 : index === 219 ? 3 : 14] });
    }
  }
  return records;
}

const CONTEXT_LEDGER_PROMPT = `Reconcile the event ledger below. This is a data-processing problem, not a request to execute code.
Each line between BEGIN_LEDGER and END_LEDGER is one JSON record.
1. Keep only records whose project is exactly ORBIT and period is exactly 2026-09. Similar names and other periods are different scopes.
2. Deduplicate these records by eventId. Repeated eventIds have identical data and count only once.
3. For each invoice in that scope, use only the record with the greatest integer revision, regardless of input order. Distinct events for one invoice have distinct revisions.
4. A latest state of void removes the invoice completely. A later posted revision restores it. A void amount is zero; it is not a negative transaction. Earlier amounts are replaced, not summed.
Return only one strict JSON object, without Markdown or commentary, with exactly these keys:
- uniqueTargetEvents: number of distinct target-scope eventIds before revision selection (an integer).
- activeInvoiceCount: number of invoices whose latest state is posted (an integer).
- totalCents: sum of the latest posted amounts, in integer cents.
- largestInvoiceId: invoice ID with the greatest latest posted amount; break a tie by lexicographically smallest ID.
- activeInvoiceIds: all active invoice IDs in ascending lexicographic order (an array of strings).
BEGIN_LEDGER
${buildContextLedgerRecords().map((record) => JSON.stringify(record)).join('\n')}
END_LEDGER`;

const CODE_TRACE_SOURCE = `const source = [2, , 0, 5];
const alias = source;
const copy = source.slice();
alias[1] = 3;
delete source[3];
let calls = 0;
const mapped = source.map((value, index) => { calls++; return (value ?? 9) + index; });
const functions = [];
for (let i = 0; i < 3; i++) functions.push(() => i);
function settle() { try { return 7; } finally { return 11; } }
console.log(JSON.stringify({source, copy, mapped, keys: Object.keys(mapped),
  calls, closures: functions.map(fn => fn()), settled: settle()}));`;

export const CHALLENGE_BENCHMARKS: readonly ScoredBenchmark[] = [
  {
    id: 'cockpitMathExactV1', suite: 'Cockpit', labelKey: 'pelican.caseKinds.mathExact',
    sourceId: 'math-exact-v1', sourceUrl: '', sourceRevision: 'local-v1', release: 'v1',
    graderVersion: 'cockpit-exact-v2',
    prompt: `A bag contains 6 distinct red balls, 5 distinct blue balls and 4 distinct green balls. Choose a uniformly random unordered set of 5 balls without replacement. Every 5-element subset is equally likely.
Call a sample valid if it contains at least one ball of each color AND strictly more red balls than blue balls.
Return only one strict JSON object with exactly these keys, without Markdown or commentary:
- validSubsets: the number of valid subsets (an integer).
- totalSubsets: the number of all 5-element subsets (an integer).
- probability: the probability that a sample is valid, as a reduced positive fraction string "p/q".
- conditionalProbability: the probability of strictly more red than blue, CONDITIONED on at least one ball of each color, as a reduced positive fraction string "p/q".
Fractions must have no spaces and use the least positive denominator.`,
    rules: [{ kind: 'jsonFields', fields: [
      { name: 'validSubsets', value: 850 }, { name: 'totalSubsets', value: 3003 },
      { name: 'probability', value: '850/3003' }, { name: 'conditionalProbability', value: '85/217' },
    ] }],
    reference: 'The valid color counts (red, blue, green) are (2,1,2) and (3,1,1). They contribute C(6,2)C(5,1)C(4,2)=450 and C(6,3)C(5,1)C(4,1)=400. There are C(15,5)=3003 samples. Inclusion-exclusion gives 2170 samples containing every color: C(15,5)-C(9,5)-C(10,5)-C(11,5)+C(6,5)+C(5,5). Thus the probabilities are 850/3003 and 850/2170=85/217.',
  },
  {
    id: 'cockpitCodeTraceV1', suite: 'Cockpit', labelKey: 'pelican.caseKinds.codeTrace',
    sourceId: 'code-trace-v1', sourceUrl: '', sourceRevision: 'local-v1', release: 'v1',
    graderVersion: 'cockpit-exact-v2',
    prompt: `Trace this ECMAScript 2022 program using standard built-ins. No prototype modifications exist. Sparse array holes are not explicit undefined values; apply the actual map, slice, Object.keys and JSON.stringify rules.
Return exactly the JSON object printed by the program. Use strict JSON only, with all seven printed keys and no extra keys, Markdown or commentary. You do not need to execute any code.
\`\`\`javascript
${CODE_TRACE_SOURCE}
\`\`\``,
    rules: [{ kind: 'jsonFields', fields: [
      { name: 'source', value: [2, 3, 0, null] }, { name: 'copy', value: [2, null, 0, 5] },
      { name: 'mapped', value: [2, 4, 2, null] }, { name: 'keys', value: ['0', '1', '2'] },
      { name: 'calls', value: 3 }, { name: 'closures', value: [0, 1, 2] }, { name: 'settled', value: 11 },
    ] }],
    reference: 'alias mutates source but not the earlier slice. Deleting index 3 preserves length and creates a hole. map skips that hole, so there are three callbacks and three own keys. JSON.stringify serializes array holes as null. Each let iteration has its own binding. A return in finally replaces the pending return from try.',
  },
  {
    id: 'cockpitContextLedgerV1', suite: 'Cockpit', labelKey: 'pelican.caseKinds.contextLedger',
    sourceId: 'context-ledger-v1', sourceUrl: '', sourceRevision: 'local-v1', release: 'v1',
    graderVersion: 'cockpit-exact-v2', prompt: CONTEXT_LEDGER_PROMPT,
    rules: [{ kind: 'jsonFields', fields: [
      { name: 'uniqueTargetEvents', value: 18 }, { name: 'activeInvoiceCount', value: 6 },
      { name: 'totalCents', value: 59323 }, { name: 'largestInvoiceId', value: 'A103' },
      { name: 'activeInvoiceIds', value: ['A102', 'A103', 'A104', 'A106', 'A107', 'A108'] },
    ] }],
    reference: 'Filter the scope before interpreting revisions. Deduplication leaves 18 target events. Latest A101 and A105 are void. The six posted invoices are A102=8100, A103=29025, A104=2199, A106=9999, A107=2500 and A108=7500 cents. Their sum is 59323. A103 is largest. Restored A103 must be included; old duplicate deliveries must not restore A101 or A105.',
  },
  {
    id: 'cockpitMultiStepV1', suite: 'Cockpit', labelKey: 'pelican.caseKinds.multiStep',
    sourceId: 'resource-planning-v1', sourceUrl: '', sourceRevision: 'local-v1', release: 'v1',
    graderVersion: 'cockpit-exact-v2',
    prompt: `Schedule eight non-preemptive jobs using a shared pool of 3 identical workers. Time starts at 0. Every start time is a nonnegative integer. A job occupies its required number of workers for its entire duration, on [start, start+duration). A successor may start exactly when all predecessors finish. Workers can be reused immediately at a finish time. No other constraints exist.
Job | duration | workers | immediate predecessors
A | 3 | 2 | none
B | 2 | 1 | none
C | 4 | 2 | A
D | 3 | 1 | B
E | 2 | 1 | A,B
F | 4 | 3 | C,D
G | 2 | 1 | E
H | 1 | 2 | F,G
Minimize the finishing time of H (all jobs are its ancestors). Among all schedules with that minimum finish, choose the lexicographically smallest start-time vector in alphabetical job order A,B,C,D,E,F,G,H. Lexicographic means minimize A's start first, then B's, and so on, subject to the optimal finish.
Return only one strict JSON object, with exactly these keys and no Markdown or commentary:
- minimumFinish: optimal finish time (integer).
- startTimes: the canonical eight-element start-time vector (array of integers).
- totalWorkerTime: sum over all jobs of duration times workers (integer).`,
    rules: [{ kind: 'jsonFields', fields: [
      { name: 'minimumFinish', value: 14 }, { name: 'startTimes', value: [0, 0, 3, 2, 5, 7, 11, 13] },
      { name: 'totalWorkerTime', value: 37 },
    ] }],
    reference: 'A valid optimal schedule starts A,B,C,D,E,F,G,H at 0,0,3,2,5,7,11,13. F needs all workers, so G cannot overlap it. If G follows F, the chain A,C,F,G,H already needs 3+4+4+2+1=14 time units. If G precedes F, A,B,C,D,E,G must all finish before F: their 23 worker-time units imply F cannot start before 8. At start 8 only one worker-time unit could be idle; enumerating the integer starts allowed by the A/B predecessors rules out this remaining case because D, C and the E-G chain compete for workers. F must then start at least at 9, followed by four units of F and one of H. An exhaustive integer-time feasibility search also rules out every finish through 13 and selects the supplied vector first at 14. Total worker time is 6+2+8+3+2+12+2+2=37.',
  },
];

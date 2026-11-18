// Differential oracle for the random grammar (two statements, one tool, JSON
// arguments, string defaults). The parser/evaluator and real upstream scheduler
// are exercised by query-scenarios.ts. This model only knows the requested keys.
import type { Dispatch, Outcome, Scenario, Step } from './query-scenarios';

const corrections = [
  'removed-call-success-on-a-recreated-key',
  'removed-call-failure-on-a-recreated-key',
  'pending-key-changes-its-only-reader',
  'idle-query-takes-over-a-pending-call',
  'rapid-edits-revisit-a-cached-result',
  'return-to-a-displaced-failed-entry',
  're-register-on-a-displaced-call',
  'return-to-a-displaced-call-after-success',
  'return-to-a-displaced-call-after-failure',
  'rapid-edits-revisit-an-outstanding-call',
  'return-to-finished-key',
  'return-to-failed-key',
  'return-to-an-outstanding-key-after-failure',
  'new-reader-of-a-retained-failure',
] as const;
type Situation = typeof corrections[number];
type Keys = Record<string, string>;
type Query = { key: string; previous?: string; loading: boolean; fetched: boolean; hidden?: Situation; error?: object };
type Entry = { data?: unknown; failed: boolean; pending: boolean; starter?: string; displaced: boolean };
type Call = { id: string; key: string; entry: Entry; dispatch: Dispatch };

function failure(id: string, outcome: Exclude<Outcome, { kind: 'result' }>) {
  const base = { source: 'query', statementId: id, component: 'Query', toolName: 't', code: outcome.kind };
  switch (outcome.kind) {
    case 'tool-not-found':
      return { ...base, message: 'Query tool "t" not found', hint: outcome.availableTools.length
        ? 'Available tools: ' + outcome.availableTools.join(', ') : undefined };
    case 'mcp-error': return { ...base, message: 'Query "t" returned an error: ' + outcome.message };
    case 'tool-error': return { ...base, message: 'Query "t" failed: ' + outcome.message };
  }
}

function simulate(scenario: Scenario, keys: Keys[], enabled: Set<Situation>) {
  const queries = new Map<string, Query>();
  const cache = new Map<string, Entry>();
  const calls: Call[] = [];
  const observations: string[] = [];
  const entryCounts: number[] = [];
  let evaluation = 0;

  function referenced(key: string) {
    return Array.from(queries.values()).some(q => q.key === key || q.previous === key);
  }
  function release(key?: string) {
    if (key !== undefined && !referenced(key)) cache.delete(key);
  }
  function evaluate(active: Keys, step: number) {
    for (const [id, q] of queries) {
      if (id in active) continue;
      queries.delete(id);
      release(q.key);
      release(q.previous);
    }
    for (const [id, key] of Object.entries(active)) {
      const existing = queries.get(id);
      const q: Query = existing ?? { key, loading: false, fetched: false };
      const entering = !existing || key !== q.key;
      const displaced = q.previous;
      if (key !== q.key) q.previous = q.key;
      q.key = key;
      queries.set(id, q);
      if (displaced !== undefined && !referenced(displaced)) {
        const entry = cache.get(displaced);
        if (entry) entry.displaced = true;
      }

      let entry = cache.get(key);
      // Trace reachability in the model. Test an abandoned entry at its next
      // lookup so the counterfactual can isolate *which* reuse was observable.
      if (entry?.displaced && entry.data === undefined && !entry.failed && !entry.pending) {
        // An empty, idle entry has no observable state to reuse.
        entry = undefined;
      }
      if (entry?.displaced) {
        let reason: Situation;
        if (entry.data !== undefined) reason = 'rapid-edits-revisit-a-cached-result';
        else if (entry.failed) reason = 'return-to-a-displaced-failed-entry';
        else if (!existing) reason = 're-register-on-a-displaced-call';
        else if (q.loading) reason = 'rapid-edits-revisit-an-outstanding-call';
        else if (q.error) reason = 'return-to-a-displaced-call-after-failure';
        else if (q.fetched) reason = 'return-to-a-displaced-call-after-success';
        else throw new Error(`unclassified displaced pending entry at ${scenario.name} step ${step}`);
        if (enabled.has(reason)) entry = undefined;
        else entry.displaced = false;
      }
      if (!entry) {
        entry = { failed: false, pending: false, displaced: false };
        cache.set(key, entry);
      }
      if (entering && entry.pending && !q.loading) {
        if (entry.starter !== id) q.hidden = 'idle-query-takes-over-a-pending-call';
        else if (!existing) q.hidden = 're-register-on-a-displaced-call';
        else if (q.error) q.hidden = 'return-to-an-outstanding-key-after-failure';
        else if (q.fetched) q.hidden = 'return-to-a-displaced-call-after-success';
        else throw new Error(`unclassified hidden loading at ${scenario.name} step ${step}`);
      }
      const needsCall = !entry.failed || (!q.error && enabled.has('new-reader-of-a-retained-failure'));
      if (entry.data === undefined && needsCall && !entry.pending) {
        entry.pending = true;
        entry.starter = id;
        q.loading = true;
        calls.push({ id, key, entry, dispatch: { step, toolName: 't', args: JSON.parse(key) } });
      }
    }
  }
  function complete(call: Call, outcome: Outcome) {
    const { entry, key, id } = call;
    const success = outcome.kind === 'result';
    if (cache.get(key) !== entry && enabled.has(success
      ? 'removed-call-success-on-a-recreated-key' : 'removed-call-failure-on-a-recreated-key')) return;
    entry.pending = false;
    const starter = queries.get(id);
    const own = starter?.key === key;
    if (own) {
      starter.loading = false;
      if (cache.get(key)?.pending) starter.hidden = success
        ? 'removed-call-success-on-a-recreated-key' : 'removed-call-failure-on-a-recreated-key';
    }
    const readers = Array.from(queries).filter(([, q]) => q.key === key);
    if (readers.length === 0) return;
    if (outcome.kind === 'result') {
      if (!own && !enabled.has('pending-key-changes-its-only-reader')) return;
      entry.data = outcome.value ?? null;
      entry.failed = false;
      for (const [, q] of readers) {
        q.error = undefined;
        q.fetched = true;
        const previous = q.previous;
        q.previous = undefined;
        release(previous);
      }
    } else {
      entry.failed = true;
      for (const [reader, q] of readers) q.error = failure(reader, outcome);
    }
  }
  function snapshot() {
    const loading: string[] = [], refetching: string[] = [], errors: object[] = [];
    const result: Record<string, unknown> = {
      __openui_loading: loading, __openui_refetching: refetching, __openui_errors: errors,
    };
    for (const [id, q] of queries) {
      const entry = cache.get(q.key)!;
      let pending = q.loading;
      if (pending && !entry.pending) {
        if (entry.data !== undefined) pending = !enabled.has('return-to-finished-key');
        else if (entry.failed) pending = !enabled.has('return-to-failed-key');
        else throw new Error('unclassified stuck loading');
      } else if (!pending && entry.pending) {
        if (!q.hidden) throw new Error(`unclassified hidden loading at ${scenario.name}`);
        pending = enabled.has(q.hidden);
      }
      if (pending) {
        loading.push(id);
        if (q.fetched) refetching.push(id);
      }
      if (q.error) errors.push(q.error);
      const prev = q.previous === undefined ? undefined : cache.get(q.previous)?.data;
      result[id] = entry.data !== undefined ? entry.data : prev !== undefined ? prev : id + 'def';
    }
    return result;
  }
  let before = JSON.stringify(snapshot());
  for (const [i, step] of scenario.steps.entries()) {
    const first = calls.length;
    if (step.kind === 'evaluate') evaluate(keys[evaluation++], i);
    if (step.kind === 'complete') {
      if (!('dispatch' in step)) throw new Error('random completions require a dispatch address');
      const call = calls.find(c => c.dispatch.step === step.dispatch.step &&
        c.key === JSON.stringify(step.dispatch.args));
      if (call) complete(call, step.outcome);
    }
    const next = snapshot();
    const bytes = JSON.stringify(next);
    observations.push(JSON.stringify({
      snapshot: next, calls: calls.slice(first).map(({ dispatch }, offset) => ({
        call: first + offset, toolName: dispatch.toolName, args: dispatch.args,
      })), changed: before !== bytes,
    }));
    entryCounts.push(new Set(Array.from(queries.values()).flatMap(q => q.previous === undefined
      ? [q.key] : [q.key, q.previous])).size);
    before = bytes;
  }
  return { observations, calls, entryCounts };
}

// Address calls by where and with which arguments they were dispatched. A
// restarted call and a released call may share a key and have distinct outcomes.
export function addressCompletions(scenario: Scenario, reference: string[], keys: Keys[], seed: number) {
  const addresses: Dispatch[] = [];
  for (const [i, bytes] of reference.entries()) {
    const step = scenario.steps[i];
    if (step.kind === 'complete' && 'call' in step) {
      scenario.steps[i] = { kind: 'complete', dispatch: addresses[step.call], outcome: step.outcome };
    }
    for (const { toolName, args } of JSON.parse(bytes).calls) addresses.push({ step: i, toolName, args });
  }
  const extra: Step[] = [];
  for (const { dispatch } of simulate(scenario, keys, new Set(corrections)).calls) {
    if (addresses.some(a => a.step === dispatch.step && JSON.stringify(a.args) === JSON.stringify(dispatch.args))) continue;
    const outcome: Outcome = (seed + dispatch.step) % 2 === 0
      ? { kind: 'result', value: [seed, dispatch.step] } : { kind: 'tool-error', message: 'offline' };
    extra.push({ kind: 'complete', dispatch, outcome });
  }
  scenario.steps.push(...extra);
  return extra.length;
}

export function compareLifetimes(scenario: Scenario, reference: string[], keys: Keys[]) {
  const { observations: expected, entryCounts } = simulate(scenario, keys, new Set(corrections));
  const baseline = simulate(scenario, keys, new Set()).observations;
  const overrides: Record<number, string> = {};
  const reasons: Record<number, string> = {};
  for (const [i, bytes] of reference.entries()) {
    if (bytes !== baseline[i]) throw new Error(`unexplained upstream observation at ${scenario.name} step ${i}\n${bytes}\n${baseline[i]}`);
    if (bytes !== expected[i]) overrides[i] = expected[i];
  }
  const differences = Object.keys(overrides).map(Number);
  if (differences.length) {
    const enabled = new Set<Situation>();
    let previous = baseline;
    for (const reason of corrections) {
      enabled.add(reason);
      const next = enabled.size === corrections.length ? expected : simulate(scenario, keys, enabled).observations;
      for (const i of differences) {
        // Multiple causes can overlap. Attribute a step to the transition out
        // of the last policy prefix that still matched the genuine reference.
        if (previous[i] === reference[i] && next[i] !== reference[i]) reasons[i] = reason;
      }
      previous = next;
    }
    for (const i of differences) {
      if (!reasons[i]) throw new Error(`unclassified difference at ${scenario.name} step ${i}`);
      const ref = JSON.parse(reference[i]), rust = JSON.parse(expected[i]);
      if (ref.changed !== rust.changed && JSON.stringify(ref.snapshot) === JSON.stringify(rust.snapshot) &&
          JSON.stringify(ref.calls) === JSON.stringify(rust.calls)) reasons[i] = 'change-flag-after-loading-converges';
    }
  }
  return { overrides, reasons, entryCounts };
}

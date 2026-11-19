// Counterfactuals for attributing differences, checked against real lang-core
// with no corrections and the independent rule model with all corrections.
// They cannot supply the Rust expectations or its cache-entry counts.
import type { Dispatch, Outcome, Scenario, Step } from './query-scenarios';
import { byRule, failure, type Keys } from './query-rule-model';

const corrections = [
  'statement-named-errors', 'statement-named-loading', 'statement-named-refetching',
  'remove-statement-before-registering-its-replacement',
  'removed-call-success-on-a-recreated-key',
  'removed-call-failure-on-a-recreated-key',
  'pending-key-changes-its-only-reader',
  'shared-entry-delivers-to-both-current-queries',
  'idle-query-takes-over-a-pending-call',
  'shared-completion-leaves-a-reader-loading',
  'handoff-failure-leaves-a-reader-loading',
  'rapid-edits-revisit-a-cached-result',
  'return-to-a-displaced-failed-entry',
  're-register-on-a-displaced-call',
  'return-to-a-displaced-call-after-success',
  'return-to-a-displaced-call-after-failure',
  'idle-reader-takes-an-abandoned-call',
  'rapid-edits-revisit-an-outstanding-call',
  'return-to-finished-key',
  'return-to-failed-key',
  'return-to-an-outstanding-key-after-failure',
  'new-reader-of-a-retained-failure',
  // Last, so that it is modelled only with every other correction applied.
  'run-through-a-query-that-did-not-start-the-call',
] as const;
type Situation = typeof corrections[number];
// Lang-core remembers a run on the query that asked (needsRefetch); the
// correction remembers it on the entry, naming the query that asked.
type Query = { key: string; previous?: string; loading: boolean; fetched: boolean; hidden?: Situation; stuck?: Situation; defaults?: unknown; error?: object; needsRefetch?: boolean };
type Entry = { data?: unknown; failed: boolean; pending: boolean; starter?: string; displaced: boolean; runAgain?: string };
type Call = { id: string; key: string; entry: Entry; dispatch: Dispatch };

function simulate(scenario: Scenario, keys: Keys[], enabled: Set<Situation>) {
  const queries = new Map<string, Query>();
  const cache = new Map<string, Entry>();
  const calls: Call[] = [];
  const observations: string[] = [];

  let evaluation = 0;

  function referenced(key: string) {
    return Array.from(queries.values()).some(q => q.key === key || q.previous === key);
  }
  // Lang-core can delete a removed statement's entry before a new statement
  // acquires it. Toggle that behaviour here to isolate its observable effects.
  function release(key?: string, acquiring?: Set<string>) {
    if (key && acquiring?.has(key) && enabled.has('remove-statement-before-registering-its-replacement')) return;
    if (key !== undefined && !referenced(key)) cache.delete(key);
  }
  function evaluate(active: Keys, step: number) {
    const acquiring = new Set(Object.values(active));
    for (const [id, q] of queries) {
      if (id in active) continue;
      queries.delete(id);
      release(q.key, acquiring);
      release(q.previous, acquiring);
    }
    for (const [id, key] of Object.entries(active)) {
      const existing = queries.get(id);
      const q: Query = existing ?? { key, loading: false, fetched: false };
      const entering = !existing || key !== q.key;
      q.defaults = listSituation(id) ? [] : id + 'def';
      if (key !== q.key) q.previous = q.key;
      q.key = key;
      queries.set(id, q);
      let entry = cache.get(key);
      // Trace reachability in the model. Test an abandoned entry at its next
      // lookup so the counterfactual can isolate *which* reuse was observable.
      if (entry?.displaced && entry.data === undefined && !entry.failed && !entry.pending) {
        // An empty, idle entry has no observable state to reuse.
        entry = undefined;
      }
      let reused: Situation | undefined;
      if (entry?.displaced) {
        let reason: Situation;
        if (entry.data !== undefined) reason = 'rapid-edits-revisit-a-cached-result';
        else if (entry.failed) reason = 'return-to-a-displaced-failed-entry';
        else if (!existing) reason = 're-register-on-a-displaced-call';
        else if (q.loading) reason = 'rapid-edits-revisit-an-outstanding-call';
        else if (q.error) reason = 'return-to-a-displaced-call-after-failure';
        else if (q.fetched) reason = 'return-to-a-displaced-call-after-success';
        else reason = 'idle-reader-takes-an-abandoned-call';
        if (enabled.has(reason)) entry = undefined;
        else {
          entry.displaced = false;
          reused = reason;
        }
      }
      if (!entry) {
        entry = { failed: false, pending: false, displaced: false };
        cache.set(key, entry);
      }
      if (entering && entry.pending && !q.loading) {
        // An abandoned call has a distinct cause from a still-live call
        // joined by an idle reader (possibly a new registration of its id).
        q.hidden = reused ?? (entry.starter === id && q.error
          ? 'return-to-an-outstanding-key-after-failure' : 'idle-query-takes-over-a-pending-call');
      }
      if (entering && !entry.pending && q.loading) {
        if (entry.data !== undefined) q.stuck = 'return-to-finished-key';
        else if (entry.failed) q.stuck = 'return-to-failed-key';
      }
      const needsCall = !entry.failed || (!q.error && enabled.has('new-reader-of-a-retained-failure'));
      if (entry.data === undefined && needsCall && !entry.pending) {
        entry.pending = true;
        entry.starter = id;
        q.loading = true;
        calls.push({ id, key, entry, dispatch: { step, toolName: 't', args: JSON.parse(key) } });
        // executeFetch rebuilds before the next statement. With a colliding
        // id this can append to a result array a second time in one command.
        rebuild();
      }
    }
  }
  // executeFetch for a query whose current entry has no call in flight.
  function start(id: string, q: Query, step: number) {
    const entry = cache.get(q.key)!;
    entry.pending = true;
    entry.starter = id;
    q.loading = true;
    calls.push({ id, key: q.key, entry, dispatch: { step, toolName: 't', args: JSON.parse(q.key) } });
    rebuild();
  }
  function invalidate(ids: string[] | undefined, step: number) {
    const targets = ids?.length ? ids.filter(id => queries.has(id)) : Array.from(queries.keys());
    for (const id of targets) {
      const q = queries.get(id)!;
      const entry = cache.get(q.key)!;
      if (!entry.pending) start(id, q, step);
      else if (enabled.has('run-through-a-query-that-did-not-start-the-call')) entry.runAgain ??= id;
      else q.needsRefetch = true;
    }
  }
  // True when it rebuilt the snapshot after its last change.
  function complete(call: Call, outcome: Outcome, step: number): boolean {
    const { entry, key, id } = call;
    const success = outcome.kind === 'result';
    if (cache.get(key) !== entry && enabled.has(success
      ? 'removed-call-success-on-a-recreated-key' : 'removed-call-failure-on-a-recreated-key')) return false;
    // Lang-core's finally refetches for the query that started the call if
    // it is still on the key. The correction starts the entry's remembered
    // run if any query is still on the key.
    const starterQuery = queries.get(id);
    let again: string | undefined;
    if (enabled.has('run-through-a-query-that-did-not-start-the-call')) {
      const asked = entry.runAgain;
      entry.runAgain = undefined;
      const readers = Array.from(queries).filter(([, q]) => q.key === key).map(([reader]) => reader);
      if (asked !== undefined && readers.length) again = readers.includes(asked) ? asked : readers[0];
    } else if (starterQuery?.key === key && starterQuery.needsRefetch) {
      starterQuery.needsRefetch = false;
      again = id;
    }
    deliver(call, outcome);
    if (again === undefined) return false;
    rebuild();
    start(again, queries.get(again)!, step);
    return true;
  }
  function deliver(call: Call, outcome: Outcome) {
    const { entry, key, id } = call;
    const success = outcome.kind === 'result';
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
    for (const [reader, q] of readers) {
      if (reader === id || !q.loading) continue;
      if (own) q.stuck = 'shared-completion-leaves-a-reader-loading';
      else q.stuck = success ? 'pending-key-changes-its-only-reader' : 'handoff-failure-leaves-a-reader-loading';
    }
    if (outcome.kind === 'result') {
      if (!own && !enabled.has('pending-key-changes-its-only-reader')) return;
      entry.data = structuredClone(outcome.value ?? null);
      entry.failed = false;
      for (const [reader, q] of readers) {
        q.error = undefined;
        if (reader !== id && own && !enabled.has('shared-entry-delivers-to-both-current-queries')) continue;
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
        if (!q.stuck) throw new Error(`unclassified stuck loading at ${scenario.name} for ${id}`);
        pending = !enabled.has(q.stuck);
      } else if (!pending && entry.pending) {
        const sharing = Array.from(queries.values()).some(other => other !== q && other.key === q.key);
        const hidden = sharing ? 'shared-entry-delivers-to-both-current-queries'
          : q.hidden ?? (entry.starter !== id ? 'idle-query-takes-over-a-pending-call' : undefined);
        if (!hidden) throw new Error(`unclassified hidden loading at ${scenario.name}`);
        pending = enabled.has(hidden);
      }
      const collision = listSituation(id);
      if (!collision || !enabled.has(collision)) {
        const prev = q.previous === undefined ? undefined : cache.get(q.previous)?.data;
        result[id] = entry.data !== undefined ? entry.data : prev !== undefined ? prev : q.defaults;
      }
      if (pending) {
        (result.__openui_loading as string[]).push(id);
        if (q.fetched) (result.__openui_refetching as string[]).push(id);
      }
      if (q.error) (result.__openui_errors as object[]).push(q.error);
    }
    return result;
  }
  let visible = snapshot(), lastJson = JSON.stringify(visible);
  function rebuild() {
    const next = snapshot(), bytes = JSON.stringify(next);
    if (bytes !== lastJson) visible = next;
    lastJson = bytes;
  }
  for (const [i, step] of scenario.steps.entries()) {
    const first = calls.length;
    const before = JSON.stringify(visible);
    if (step.kind === 'evaluate') { evaluate(keys[evaluation++], i); rebuild(); }
    if (step.kind === 'complete') {
      if (!('dispatch' in step)) throw new Error('random completions require a dispatch address');
      const call = calls.find(c => c.dispatch.step === step.dispatch.step &&
        c.key === JSON.stringify(step.dispatch.args));
      if (call && !complete(call, step.outcome, i)) rebuild();
    }
    // Lang-core's invalidate rebuilds only inside each executeFetch.
    if (step.kind === 'invalidate') invalidate(step.ids, i);
    for (const [key, entry] of cache) if (!referenced(key)) entry.displaced = true;
    const bytes = JSON.stringify(visible);
    observations.push(JSON.stringify({
      snapshot: visible, calls: calls.slice(first).map(({ dispatch }, offset) => ({
        call: first + offset, toolName: dispatch.toolName, args: dispatch.args,
      })), changed: before !== bytes,
    }));
  }
  return observations;
}

function listSituation(id: string): Situation | undefined {
  switch (id) {
    case '__openui_errors': return 'statement-named-errors';
    case '__openui_loading': return 'statement-named-loading';
    case '__openui_refetching': return 'statement-named-refetching';
  }
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
  for (const dispatch of byRule(scenario, keys).calls) {
    if (addresses.some(a => a.step === dispatch.step && JSON.stringify(a.args) === JSON.stringify(dispatch.args))) continue;
    const outcome: Outcome = (seed + dispatch.step) % 2 === 0
      ? { kind: 'result', value: [seed, dispatch.step] } : { kind: 'tool-error', message: 'offline' };
    extra.push({ kind: 'complete', dispatch, outcome });
  }
  scenario.steps.push(...extra);
  return extra.length;
}

export function compareLifetimes(scenario: Scenario, reference: string[], keys: Keys[]) {
  const { observations: expected, entryCounts } = byRule(scenario, keys);
  const baseline = simulate(scenario, keys, new Set());
  const corrected = simulate(scenario, keys, new Set(corrections));
  const overrides: Record<number, string> = {};
  const reasons: Record<number, string> = {};
  for (const [i, bytes] of reference.entries()) {
    if (bytes !== baseline[i]) throw new Error(`unexplained upstream observation at ${scenario.name} step ${i}\n${bytes}\n${baseline[i]}`);
    if (corrected[i] !== expected[i]) throw new Error(`counterfactual disagrees with rules at ${scenario.name} step ${i}\n${corrected[i]}\n${expected[i]}`);
    if (bytes !== expected[i]) overrides[i] = expected[i];
  }
  const differences = Object.keys(overrides).map(Number);
  if (differences.length) {
    const enabled = new Set<Situation>();
    let previous = baseline;
    for (const reason of corrections) {
      enabled.add(reason);
      const next = enabled.size === corrections.length ? corrected : simulate(scenario, keys, enabled);
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

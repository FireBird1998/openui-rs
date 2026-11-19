// Expected Rust behaviour for the random grammar. Compute holders from the
// final registrations of a command, independently of upstream release points,
// the runtime's reference counts and the counterfactual attribution traces.
import type { Dispatch, Outcome, Scenario } from './query-scenarios';

export type Keys = Record<string, string>;
export type WaitFor = Record<string, string[]>;
type Query = { waiting?: boolean; key: string; previous?: string; fetched: boolean; error?: object };
// A run asked for while a call is in flight belongs to the entry.
type Entry = { data?: unknown; failed?: boolean; pending?: Dispatch; runAgain?: boolean };

export function failure(id: string, outcome: Exclude<Outcome, { kind: 'result' }>) {
  const base = { source: 'query', statementId: id, component: 'Query', toolName: 't', code: outcome.kind };
  switch (outcome.kind) {
    case 'tool-not-found':
      return { ...base, message: 'Query tool "t" not found', hint: outcome.availableTools.length
        ? 'Available tools: ' + outcome.availableTools.join(', ') : undefined };
    case 'mcp-error': return { ...base, message: 'Query "t" returned an error: ' + outcome.message };
    case 'tool-error': return { ...base, message: 'Query "t" failed: ' + outcome.message };
  }
}

function heldKeys(queries: Iterable<Query>) {
  const held = new Set<string>();
  for (const q of queries) {
    held.add(q.key);
    if (q.previous !== undefined) held.add(q.previous);
  }
  return held;
}

export function byRule(scenario: Scenario, keys: Keys[], waitFor: WaitFor) {
  const queries = new Map<string, Query>();
  let entries = new Map<string, Entry>();
  const calls: Dispatch[] = [], observations: string[] = [], entryCounts: number[] = [];
  let evaluation = 0;
  function snapshot() {
    const loading: string[] = [], refetching: string[] = [], errors: object[] = [];
    const out: Record<string, unknown> = {
      __openui_loading: loading, __openui_refetching: refetching, __openui_errors: errors,
    };
    for (const [id, q] of queries) {
      const entry = entries.get(q.key)!;
      if (entry.pending) {
        loading.push(id);
        if (q.fetched) refetching.push(id);
      }
      if (q.error) errors.push(q.error);
      if (!Object.hasOwn(out, id)) {
        const prev = entries.get(q.previous ?? '')?.data;
        out[id] = entry.data !== undefined ? entry.data : prev !== undefined ? prev : id + 'def';
      }
    }
    return out;
  }
  let before = JSON.stringify(snapshot());
  for (const [i, step] of scenario.steps.entries()) {
    const first = calls.length;
    if (step.kind === 'evaluate') {
      const active = keys[evaluation++];
      for (const id of queries.keys()) if (!(id in active)) queries.delete(id);
      for (const [id, key] of Object.entries(active)) {
        const q: Query = queries.get(id) ?? { key, fetched: false };
        if (key !== q.key) q.previous = q.key;
        q.key = key;
        queries.set(id, q);
      }
      // The old map supplies the exact same entry to any surviving key,
      // including a key acquired by an entirely new statement.
      entries = new Map(Array.from(heldKeys(queries.values()), k => [k, entries.get(k) ?? {}]));
      for (const [id, key] of Object.entries(active)) {
        const entry = entries.get(key)!;
        const q = queries.get(id)!;
        q.waiting = waitFor[id]?.some(dep => {
          const source = queries.get(dep);
          return entries.get(source?.key ?? '')?.data === undefined &&
            entries.get(source?.previous ?? '')?.data === undefined;
        });
        if (q.waiting || entry.data !== undefined || entry.pending || (entry.failed && queries.get(id)!.error)) continue;
        entry.pending = { step: i, toolName: 't', args: JSON.parse(key) };
        calls.push(entry.pending);
      }
    } else if (step.kind === 'complete') {
      if (!('dispatch' in step)) throw new Error('expected dispatch address');
      const key = JSON.stringify(step.dispatch.args);
      const entry = entries.get(key);
      if (entry?.pending && entry.pending.step === step.dispatch.step) {
        entry.pending = undefined;
        for (const [id, q] of queries) {
          if (q.key !== key) continue;
          if (step.outcome.kind === 'result') {
            entry.data = step.outcome.value ?? null;
            entry.failed = false;
            q.error = undefined;
            q.previous = undefined;
            q.fetched = true;
          } else {
            entry.failed = true;
            q.error = failure(id, step.outcome);
          }
        }
        // The delivered call starts the remembered run if a query is still
        // on the key; otherwise the run is forgotten.
        if (entry.runAgain) {
          entry.runAgain = false;
          if (Array.from(queries.values()).some(q => q.key === key)) {
            entry.pending = { step: i, toolName: 't', args: JSON.parse(key) };
            calls.push(entry.pending);
          }
        }
        const held = heldKeys(queries.values());
        for (const key of entries.keys()) if (!held.has(key)) entries.delete(key);
      }
    } else if (step.kind === 'invalidate') {
      const targets = step.ids?.length ? step.ids.filter(id => queries.has(id)) : Array.from(queries.keys());
      for (const id of targets) {
        if (queries.get(id)!.waiting) continue;
        const entry = entries.get(queries.get(id)!.key)!;
        if (entry.pending) entry.runAgain = true;
        else {
          entry.pending = { step: i, toolName: 't', args: JSON.parse(queries.get(id)!.key) };
          calls.push(entry.pending);
        }
      }
    }
    const next = snapshot(), bytes = JSON.stringify(next);
    observations.push(JSON.stringify({
      snapshot: next, calls: calls.slice(first).map(({ toolName, args }, offset) => ({ call: first + offset, toolName, args })),
      changed: bytes !== before,
    }));
    before = bytes;
    entryCounts.push(entries.size);
  }
  return { observations, calls, entryCounts };
}

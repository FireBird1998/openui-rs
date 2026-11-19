// Counterfactuals run the vendored evaluator, promises and round loop. Each
// switch changes a cache/reader decision at the point named by its table row.
import {readFileSync} from 'node:fs';

import type {createQueryManager} from './lang-core/src/runtime/queryManager';

export const dispatchRow = 'a-reader-only-dispatches-keys-still-current-after-the-cascade';
export const rows = [
  'a-seeded-self-reader-does-not-follow-its-own-result',
  'remove-statement-before-registering-its-replacement',
  'rapid-edits-revisit-a-cached-result',
  'return-to-a-displaced-failed-entry',
  'rapid-edits-revisit-an-outstanding-call',
  're-register-on-a-displaced-call',
  'idle-reader-takes-an-abandoned-call',
  'return-to-a-displaced-call-after-success',
  'return-to-a-displaced-call-after-failure',
  'removed-call-success-on-a-recreated-key',
  'removed-call-failure-on-a-recreated-key',
  'pending-key-changes-its-only-reader',
  'shared-entry-delivers-to-both-current-queries',
  'new-reader-of-a-retained-failure',
  'shared-completion-leaves-a-reader-loading',
  'handoff-failure-leaves-a-reader-loading',
  'return-to-finished-key',
  'return-to-failed-key',
  'return-to-an-outstanding-key-after-failure',
  'idle-query-takes-over-a-pending-call',
  'a-waiting-reader-leaves-its-own-pending-key',
  'reader-of-a-sharing-query-is-pending',
  'run-through-a-query-that-did-not-start-the-call',
  'a-previous-key-releases-a-waiting-self-reader',
  dispatchRow,
] as const;
export type RoundTrace = {
  rounds: string[][]; left: string[]
};
export async function readerPolicies() {
  const url = new URL('./lang-core/src/runtime/queryManager.ts', import.meta.url);
  let code = readFileSync(url, 'utf8');
  function edit(from: string, to: string) {
    if (code.split(from).length !== 2) throw Error('unexpected queryManager.ts: ' + from);
    code = code.replace(from, () => to);
  }
  // Keep the first request per key, then call the real provider for the final
  // holders in that order. Earlier commands' in-flight calls stay upstream's.
  edit('  const queries =', `  const requested = new Map();
  function scheduleRequest(key, id, asked) {
    if (!allow('${dispatchRow}')) return executeFetch(key, id, asked);
    if (!requested.has(key)) requested.set(key, {id, asked});
  }
  function dispatchCurrent() {
    for (const [key, {id, asked}] of requested) {
      const holder = queries.get(id)?.cacheKey === key ? id :
        [...queries].find(([,q]) => q.cacheKey === key)?.[0];
      if (holder !== undefined && !cache.get(key)?.inFlight) executeFetch(key, holder, asked);
    }
    requested.clear();
  }
  const queries =`);
  edit('executeFetch(cacheKey, node.statementId);',
       'scheduleRequest(cacheKey, node.statementId, {toolName:q.toolName,args:q.args});');
  edit('executeFetch(current.cacheKey, statementId);',
       'scheduleRequest(current.cacheKey, statementId, {toolName:current.toolName,args:current.args});');
  const stale = 'getData(id) !== seen.get(s.statementId)?.get(id)';
  const staleRead = `(id !== s.statementId || !allow('${rows[0]}') ? ${stale} :
    allow('a-previous-key-releases-a-waiting-self-reader') ?
      queries.get(id)?.selfMissing && getData(id) !== undefined :
      queries.get(id)?.waiting && seen.get(id)?.get(id) === undefined &&
        cache.get(queries.get(id)?.cacheKey)?.data === undefined && ${stale})`;
  edit(stale, staleRead);
  edit('if (q.waiting) continue;', `q.selfMissing = q.waitFor?.includes(node.statementId) && getData(node.statementId) === undefined;
      q.otherMissing = q.waitFor?.some(id => id !== node.statementId && getData(id) === undefined);
      if (q.waiting) continue;`);
  edit('function reevaluateReaders() {', `function reevaluateReaders() {
    const trace = observe?.();`);
  edit('if (stale.size === 0) return;', `if (stale.size === 0) break;
    trace?.rounds.push([...stale].map(s=>s.statementId));`);
  edit(
      '  }\n\n  function evaluateQueries',
      `    if(trace) trace.left = statements.filter(s=>s.waitFor?.some(id=>
    ${staleRead})).map(s=>s.statementId);
    dispatchCurrent();
    // Displacement is decided after the whole trigger, including every round.
    for(const [key,e] of cache) {
      e.displaced = ![...queries.values()].some(q=>q.cacheKey===key || q.prevCacheKey===key);
      if(e.removal && e.displaced) cache.delete(key);
      e.removal=false;
    }
  }\n\n  function evaluateQueries`);
  edit(
      'cleanupCacheEntry(q.cacheKey);\n        if (q.prevCacheKey) cleanupCacheEntry(q.prevCacheKey);',
      `if (!allow('remove-statement-before-registering-its-replacement')) {
      cleanupCacheEntry(q.cacheKey); if(q.prevCacheKey) cleanupCacheEntry(q.prevCacheKey);
    } else { for(const k of [q.cacheKey,q.prevCacheKey]) if(cache.has(k) && ![...queries.values()].some(holder=>holder.cacheKey===k || holder.prevCacheKey===k)) cache.get(k).removal = true; }`);
  edit('const existing = queries.get(node.statementId);', `const existing = queries.get(node.statementId);
    const prior = cache.get(cacheKey);
    if(prior?.displaced) {
      const reason = prior.data!==undefined ? 'rapid-edits-revisit-a-cached-result' :
        prior.failed ? 'return-to-a-displaced-failed-entry' :
        !existing ? 're-register-on-a-displaced-call' :
        existing.loading ? 'rapid-edits-revisit-an-outstanding-call' :
        existing.error ? 'return-to-a-displaced-call-after-failure' :
        existing.everFetched ? 'return-to-a-displaced-call-after-success' : 'idle-reader-takes-an-abandoned-call';
      if(allow(reason)) cache.delete(cacheKey);
    }
    if(prior) { prior.displaced=false; prior.removal=false; }
    const entering = !existing || existing.cacheKey !== cacheKey;`);
  edit('q.waitFor = node.waitFor;', `if(entering) {
      const e=cache.get(cacheKey);
      q.hidden=e?.inFlight && !q.loading ? (queries.get(e.starter)?.cacheKey===cacheKey && !existing ? 'shared-entry-delivers-to-both-current-queries' : e.starter===node.statementId && q.error ?
        'return-to-an-outstanding-key-after-failure':'idle-query-takes-over-a-pending-call') : undefined;
      q.stuck = q.loading && !e?.inFlight ? (e?.data!==undefined ? 'return-to-finished-key' :
        e?.failed ? 'return-to-failed-key' : undefined) : undefined;
    }
    q.waitFor = node.waitFor;`);
  edit('entry.failed);', `(entry.failed && (q.error || !allow('new-reader-of-a-retained-failure'))));`);
  edit(
      'q.loading = true;',
      'q.loading = true; entry.starter = statementId; q.hidden=undefined; q.stuck=undefined;');
  edit(
      'if (!current || current.cacheKey !== fetchKey) {',
      `if(cache.get(fetchKey)!==entry && allow('removed-call-success-on-a-recreated-key')) return;
    if ((!current || current.cacheKey !== fetchKey) && !(allow('pending-key-changes-its-only-reader') &&
      [...queries.values()].some(q=>q.cacheKey===fetchKey))) {`);
  edit('current.everFetched = true;', `for(const [sid,holder] of queries) if(holder.cacheKey===fetchKey &&
      (sid===statementId || allow(current?.cacheKey===fetchKey ? 'shared-entry-delivers-to-both-current-queries' : 'pending-key-changes-its-only-reader'))) {
      holder.everFetched=true;
      if (holder.selfMissing && allow('a-previous-key-releases-a-waiting-self-reader')) {
        holder.selfMissing=false; holder.waiting=holder.otherMissing;
      }
      if(sid!==statementId && holder.prevCacheKey) {
        const prev=holder.prevCacheKey; holder.prevCacheKey=undefined; cleanupCacheEntry(prev);
      }
    }`);
  edit(
      'if (current.prevCacheKey && current.prevCacheKey !== fetchKey)',
      'if (current?.cacheKey===fetchKey && current.prevCacheKey && current.prevCacheKey !== fetchKey)');
  edit(
      'const failure =',
      `if(cache.get(fetchKey)!==entry && allow('removed-call-failure-on-a-recreated-key')) return;
    if(![...queries.values()].some(q=>q.cacheKey===fetchKey) && allow('new-reader-of-a-retained-failure')) return;
    const failure =`);
  edit('entry.inFlight = false;\n      const current', `entry.inFlight = false;
    for(const [sid,q] of queries) if(sid!==statementId && q.cacheKey===fetchKey && q.loading)
      q.stuck = queries.get(statementId)?.cacheKey===fetchKey ? 'shared-completion-leaves-a-reader-loading' :
         entry.failed ? 'handoff-failure-leaves-a-reader-loading' : 'pending-key-changes-its-only-reader';
    const current`);
  edit('return q.loading || (!!q.waiting', `const e=cache.get(q.cacheKey);
    let own=q.loading;
    if(q.waiting && !e?.inFlight && allow('a-waiting-reader-leaves-its-own-pending-key')) own=false;
    if(q.stuck && allow(q.stuck) && !e?.inFlight) own=false;
    if(!own && e?.inFlight && allow(q.hidden ?? 'shared-entry-delivers-to-both-current-queries')) own=true;
    if(q.waiting && e?.inFlight && allow('reader-of-a-sharing-query-is-pending')) own=true;
    return own || (!!q.waiting`);
  // ADR 0013 attaches one remembered request to the entry. Use a current
  // holder for delivery and keep the first asker's tool and arguments.
  edit('q.needsRefetch = true;', `if (allow('run-through-a-query-that-did-not-start-the-call')) {
      entry.again ??= {toolName:q.toolName,args:q.args};
    } else q.needsRefetch = true;`);
  edit('async function executeFetch(cacheKey: string, statementId: string)',
       'async function executeFetch(cacheKey: string, statementId: string, asked?)');
  edit('const toolName = q.toolName;\n    const args = q.args;',
       'const { toolName, args } = asked ?? q;');
  edit('      reevaluateReaders();', `      if (entry.again) {
        const request = entry.again;
        entry.again = undefined;
        const holder = [...queries].find(([,q]) => q.cacheKey === fetchKey);
        if (holder && cache.get(fetchKey) === entry) scheduleRequest(fetchKey, holder[0], request);
      }
      reevaluateReaders();`);
  code +=
      '\nlet enabled = new Set(), observe; function allow(row){return enabled.has(row)} export function configure(rows, observer){enabled=new Set(rows);observe=observer;}';
  code = code.replace(/from "(\.[^"]+)"/g, (_, name) => 'from "' + new URL(name + '.ts', url).href + '"');
  const js = new Bun.Transpiler({loader: 'ts'}).transformSync(code);
  const mod = await import('data:text/javascript;base64,' + Buffer.from(js).toString('base64'));
  return (count: number|string[], observer?: () => RoundTrace): typeof createQueryManager => {
    mod.configure(typeof count === 'number' ? rows.slice(0, count) : count, observer);
    return mod.createQueryManager;
  };
}

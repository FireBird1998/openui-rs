// Observe rule 2 counterfactually. Change only the source of a query's own
// pending flag inside upstream isPending; its waiting graph, reader evaluator,
// cache, promises and snapshot change detection all run in the same manager.
import { readFileSync } from 'node:fs';

export async function withEntryLoading(includeFinished: boolean) {
  const source = new URL('./lang-core/src/runtime/queryManager.ts', import.meta.url);
  const original = readFileSync(source, 'utf8');
  const flag = 'return q.loading || (!!q.waiting';
  if (original.split(flag).length !== 2) throw new Error('unexpected upstream isPending');
  const condition = includeFinished ? '(q.waiting || cache.get(q.cacheKey)?.data !== undefined)' : 'q.waiting';
  const code = original.replace(flag, `return (${condition} ? !!cache.get(q.cacheKey)?.inFlight : q.loading) || (!!q.waiting`)
    .replace(/from "(\.[^"]+)"/g, (_, name) => 'from "' + new URL(name + '.ts', source).href + '"');
  const js = new Bun.Transpiler({loader: 'ts'}).transformSync(code);
  return (await import('data:text/javascript;base64,' + Buffer.from(js).toString('base64'))).createQueryManager;
}

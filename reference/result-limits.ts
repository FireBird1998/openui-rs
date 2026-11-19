// Run from reference/: bun --no-install result-limits.ts
// Tool result limits (#90) through lang-core's own query manager. Upstream
// has no limits: it ignores each scenario's `limits`, so every refusal is a
// pinned divergence in conformance/divergences/result-limit-*.json.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { replay, withFakeClock, type Scenario } from './query-scenarios';

await withFakeClock(async () => {
  const scenarios: Scenario[] = JSON.parse(readFileSync('../conformance/result-limits.json', 'utf8'));
  const output = [];
  for (const scenario of scenarios) output.push([scenario.name, await replay(scenario)]);
  mkdirSync('../conformance/golden', { recursive: true });
  writeFileSync('../conformance/golden/result-limits.json', JSON.stringify(Object.fromEntries(output)) + '\n');
});

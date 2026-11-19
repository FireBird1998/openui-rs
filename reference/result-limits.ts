// Run from reference/: bun --no-install result-limits.ts
// Tool result limits (#90) through lang-core's own query manager. Upstream
// has no limits: it ignores each scenario's `limits`, so every refusal is a
// pinned divergence in conformance/divergences/result-limit-*.json.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { replay, withFakeClock, type Scenario } from './query-scenarios';

await withFakeClock(async () => {
  const scenarios: (Scenario & { failureControl?: { step: number; message: string } })[] = JSON.parse(readFileSync('../conformance/result-limits.json', 'utf8'));
  const output = [];
  for (const scenario of scenarios) {
    output.push([scenario.name, await replay(scenario)]);
    // A second run rejects the same promise with the admission message.
    // Compare what readers do independently of upstream's lack of a byte cap.
    const control = scenario.failureControl;
    if (control) {
      const steps = scenario.steps.map((step, i) => i === control.step && step.kind === 'complete'
        ? { ...step, outcome: { kind: 'tool-error' as const, message: control.message } } : step);
      output.push([scenario.name + '-provider-failure', await replay({ ...scenario, steps })]);
    }
  }
  mkdirSync('../conformance/golden', { recursive: true });
  writeFileSync('../conformance/golden/result-limits.json', JSON.stringify(Object.fromEntries(output)) + '\n');
});

// Run from reference/: bun --no-install core-divergences.ts
// Engine messages and native-function text are intentionally pinned: regeneration
// always uses Bun (JavaScriptCore). No recursion-limit or allocation-limit probes.
import {readFileSync, writeFileSync} from 'fs';
import {join} from 'path';
import {parseMessage} from './lang-core/src/message';
process.env.OPENUI_TELEMETRY_DISABLED = '1';
process.env.DO_NOT_TRACK = '1';
const {compileSchema, parse} = await import('./lang-core/src/parser/parser');
const base = join(import.meta.dir, '..', 'conformance');
const {cases} = JSON.parse(readFileSync(join(base, 'core-divergences.json'), 'utf8'));
const results = {};
for (const c of cases) {
  // The committed inputs are small; never turn this into an OOM/stack-limit hunt.
  if (Buffer.byteLength(JSON.stringify(c)) > 16384) throw Error('oversized core divergence case');
  try {
    if (c.operation === 'parse') {
      results[c.name] = {output: JSON.stringify(parse(c.program, compileSchema(c.schema)))};
    } else if (c.operation === 'message') {
      results[c.name] = {output: JSON.stringify(parseMessage(c.input))};
    } else if (c.operation === 'json') {
      const value = JSON.parse(c.input);
      results[c.name] = {output: Number.isFinite(value) || typeof value !== 'number'
        ? JSON.stringify(value) : String(value)};
    } else throw Error('unknown operation');
  } catch (e) {
    results[c.name] = {throw: {name: e.name, message: e.message}};
  }
}
writeFileSync(join(base, 'golden/core-divergences.json'), JSON.stringify(results) + '\n');

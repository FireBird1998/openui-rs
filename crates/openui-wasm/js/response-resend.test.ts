import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { wasm } from './support';

// ADR 0015: the package remembers the last text the session's reader
// accepted and sends only what follows it. Should the two ever disagree,
// the reader refuses the append and the package sends the whole text once.
// The package captures WeakMap.prototype.set when it loads, so a second
// copy loaded under a patched set remembers each text one unit short: every
// append it sends then names the wrong length.
const originalSet = WeakMap.prototype.set;
WeakMap.prototype.set = function (key, value) {
  return originalSet.call(this, key, typeof value === 'string' && value.length > 0 ? value.slice(0, -1) : value);
};
const stale = await import('../../../target/wasm-pkg/web/openui_wasm.js');
WeakMap.prototype.set = originalSet;
stale.initSync({ module: readFileSync(new URL('../../../target/wasm-pkg/web/openui_wasm_bg.wasm', import.meta.url)) });

test('a reader that accepted another text than the package remembers gets the whole text', () => {
  const schema = '{"$defs":{"Box":{"properties":{"value":{}}}}}';
  using library = new stale.ComponentLibrary(schema);
  using session = new stale.RuntimeSession(library);
  using control = new wasm.ComponentLibrary(schema);
  const text = ']]>openui:content?name=x\nrows=[\n  Box("a")\n]\nroot=Box(rows)\n]]>openui:end\n';
  for (let at = 1; at <= text.length; at += 3) {
    const prefix = text.slice(0, at);
    const { evaluation, name } = session.setResponse(prefix, at < text.length);
    using output = evaluation;
    using fresh = new wasm.RuntimeSession(control);
    const expected = fresh.setResponse(prefix, at < text.length);
    using expectedOutput = expected.evaluation;
    expect([output.json, name]).toEqual([expectedOutput.json, expected.name]);
  }
});

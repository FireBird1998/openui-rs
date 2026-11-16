// Both generated targets must load with the complete application guard.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const node = require('../target/wasm-pkg/node/openui_wasm.js');
const web = await import('../target/wasm-pkg/web/openui_wasm.js');
web.initSync({ module: readFileSync(new URL('../target/wasm-pkg/web/openui_wasm_bg.wasm', import.meta.url)) });
for (const [name, pkg] of [['node', node], ['web', web]]) {
  const library = new pkg.ComponentLibrary('{"$defs":{}}');
  const session = new pkg.RuntimeSession(library);
  if (session.snapshot() !== '{}') throw new Error(`${name}: unexpected initial state`);
  session.free(); library.free();
  try { session.version(); throw new Error(`${name}: freed session accepted a call`); }
  catch (error) {
    if (error.kind !== 'Disposed' || error.message !== 'object freed') throw error;
  }
  console.log(`${name}: package loaded and guarded session checked`);
}

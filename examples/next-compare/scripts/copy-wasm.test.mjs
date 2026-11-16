import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, copyFileSync, readFileSync, writeFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
const files = ['openui_wasm.js', 'openui_wasm.d.ts', 'openui_wasm_bg.wasm', 'openui_wasm_bg.wasm.d.ts'];
function fixture(body) {
  const root = mkdtempSync(join(tmpdir(), 'openui-copy-'));
  const app = join(root, 'examples/next-compare');
  mkdirSync(join(app, 'scripts'), { recursive: true });
  const script = join(app, 'scripts/copy-wasm.mjs');
  copyFileSync(new URL('./copy-wasm.mjs', import.meta.url), script);
  try { body({ root, app, run: () => spawnSync(process.execPath, [script], { encoding: 'utf8', env: { ...process.env, PATH: '' } }) }); }
  finally { rmSync(root, { recursive: true, force: true }); }
}
test('copies the existing package without any build tools on PATH', () => fixture(({ root, app, run }) => {
  const built = join(root, 'target/wasm-pkg/web');
  mkdirSync(built, { recursive: true });
  for (const file of files) writeFileSync(join(built, file), `built ${file}`);
  const result = run();
  assert.equal(result.status, 0, result.stderr);
  for (const file of files) assert.equal(readFileSync(join(app, 'wasm', file), 'utf8'), `built ${file}`);
}));
test('missing build fails clearly before copying any artifacts', () => fixture(({ app, run }) => {
  const result = run();
  assert.equal(result.status, 1);
  assert.match(result.stderr, /WebAssembly build is missing/);
  assert.match(result.stderr, /Build it first, from the repo root:[\s\S]*scripts\/wasm.sh/);
  assert.equal(existsSync(join(app, 'wasm')), false);
}));

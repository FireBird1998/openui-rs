// Offline observations for #63 and #71. Run from the repository root:
// cd reference && bun --no-install ../tools/viewer/Checks/validation-reference.ts [--write]
import { writeFileSync, readFileSync } from 'node:fs';
import { parseStructuredRules, validate } from '../../../reference/lang-core/src/utils/validation';
const cases: [unknown, unknown, boolean][] = [
  [{url: true}, 'http://a.+4', false],
  [{url: true}, 'http://xn--a.com', true],
  [{url: true}, 'http://xn--bcher-kva.com', false],
  [{url: true}, 'http://xn--bcher-kva.a"b', false],
  [{pattern: '^\\d+$'}, '١٢', true], [{pattern: '^\\d+$'}, '12', false],
  [{pattern: '^[\\d]+$'}, '١٢', true], [{pattern: '^a$'}, 'a\n', true],
  [{email: true}, 'a@b.co\n', true], [{url: true}, 'http://user@', true],
  [{url: true}, 'http://host:99999', true], [{url: true}, '\u00a0http://host', true],
  [{url: true}, ' http://user@host:65535 ', false], [['required'], '', false],
  [{url: true}, 'http:\r\n//host', false], [{url: true}, 'http://host:65536', true],
  [{"pattern": "^\\w+$"}, "é", true],
  [{"pattern": "^\\w+$"}, "a_Z9", false],
  [{"pattern": "^\\W+$"}, "é", false],
  [{"pattern": "\\p{L}"}, "é", true],
  [{"pattern": "\\p{L}"}, "p{L}", false],
  [{"pattern": "[]a]"}, "a]", true],
  [{"pattern": "[[a]]"}, "a", true],
  [{"pattern": "[[a]]"}, "a]", false],
  [{"pattern": "[[a]]"}, "[]", false],
  [{"pattern": "(?i)a"}, "b", false],
  [{"url": true}, "http://a\"b", false],
  [{"url": true}, "http://a`b", false],
  [{"url": true}, "http://a{b}", false],
  [{"url": true}, "http://999.999.999.999", true],
  [{"url": true}, "http://1.2.3.4.5", true],
  [{"url": true}, "http://a%20b.com", true],
  [{"url": true}, "http://127.1", false],
  [{"url": true}, "http://0x7f.1", false],
  [{"url": true}, "http://256.1", true],
  [{"pattern": "^\\d+$"}, "١", true],
  [{"pattern": "^\\D+$"}, "١", false],
  [{"pattern": "^\\s$"}, "﻿", false],
  [{"pattern": "^\\s$"}, "", true],
  [{"pattern": "^\\s$"}, " ", false],
  [{"pattern": "^\\S$"}, "", false],
  [{"pattern": "\\bé\\b"}, "é", true],
  [{"pattern": "\\ba\\b"}, "éaé", false],
  [{"pattern": "\\Bé\\B"}, "é", false],
  [{"pattern": "^[\\w]+$"}, "é", true],
  [{"pattern": "^[\\s]+$"}, "﻿", false],
  [{"pattern": "[\\b]"}, "\b", false],
  [{url: true}, 'http://0xffffffffffffffffffffffff', true],
  [{url: true}, 'http://0x100000000', true],
  [{url: true}, 'http://0377.0.0.1', false],
  [{url: true}, 'http://08', true],
  [{pattern: '^\\p{L}$'}, 'p{L}', false],
  [{email: true}, 'a\ufeff@b.co', true],
  [{email: true}, 'a\u0085@b.co', false],
  [{pattern: '^[\u0301\\w]+$'}, 'é', true],
  [{pattern: '(?i:a)'}, 'A', false],
  [{pattern: '(?i:a)'}, 'b', true],
];
// Still-open engine/IDNA differences; native answers are pinned separately.
const remainders: [unknown, unknown, boolean, boolean][] = [
  [{pattern: '^.$'}, '😀', true, false],
  [{pattern: '^a\\z'}, 'a', true, false],
  [{url: true}, 'http://a%C2%A0b.com', true, false],
  [{pattern: '(?i:^\\w+$)'}, 'K', true, false],
];
for (const [rules, value, blocks] of [...cases, ...remainders]) {
  const actual = validate(value, parseStructuredRules(rules)) !== undefined;
  if (actual !== blocks) throw new Error(JSON.stringify({rules, value, blocks, actual}));
}
const observed = cases.map(([rules, value, blocks]) => ({rules, value, blocks}));
const fixture = new URL('./validation-observations.json', import.meta.url);
if (process.argv.includes('--write')) writeFileSync(fixture, JSON.stringify(observed, null, 2) + '\n');
else if (JSON.stringify(JSON.parse(readFileSync(fixture, 'utf8'))) !== JSON.stringify(observed)) {
  throw new Error('offline observations differ; regenerate with --write');
}
const remainderFixture = new URL('./validation-remainders.json', import.meta.url);
const remainderObservations = remainders.map(([rules, value, blocks, nativeBlocks]) => ({rules, value, blocks, nativeBlocks}));
if (process.argv.includes('--write')) writeFileSync(remainderFixture, JSON.stringify(remainderObservations, null, 2) + '\n');
else if (JSON.stringify(JSON.parse(readFileSync(remainderFixture, 'utf8'))) !== JSON.stringify(remainderObservations)) throw Error('remainder observations differ');
console.log(`${cases.length} upstream validation observations and ${remainders.length} documented remainders passed`);

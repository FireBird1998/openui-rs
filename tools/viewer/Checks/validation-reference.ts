// Offline observations for #63. Run from the repository root:
// bun --no-install tools/viewer/Checks/validation-reference.ts
import { parseStructuredRules, validate } from '../../../reference/lang-core/src/utils/validation';
const cases: [unknown, unknown, boolean][] = [
  [{pattern: '^\\d+$'}, '١٢', true], [{pattern: '^\\d+$'}, '12', false],
  [{pattern: '^[\\d]+$'}, '١٢', true], [{pattern: '^a$'}, 'a\n', true],
  [{email: true}, 'a@b.co\n', true], [{url: true}, 'http://user@', true],
  [{url: true}, 'http://host:99999', true], [{url: true}, '\u00a0http://host', true],
  [{url: true}, ' http://user@host:65535 ', false], [['required'], '', false],
  [{url: true}, 'http:\r\n//host', false], [{url: true}, 'http://host:65536', true],
];
for (const [rules, value, blocks] of cases) {
  const actual = validate(value, parseStructuredRules(rules)) !== undefined;
  if (actual !== blocks) throw new Error(JSON.stringify({rules, value, blocks, actual}));
}
console.log(`${cases.length} upstream validation observations passed`);

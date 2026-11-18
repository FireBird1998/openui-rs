// Run from reference/: bun --no-install mcp-results.ts
import { writeFileSync } from 'node:fs';
import { extractToolResult, McpToolError } from './lang-core/src/runtime/mcp';

const text = (text: unknown) => ({ type: 'text', text });
const content = (...parts: unknown[]) => ({ content: parts });
const error = (...parts: unknown[]) => ({ isError: true, ...content(...parts) });
const image = { type: 'image', text: 'ignored' };
const cases: [string, string][] = [];
const add = (name: string, input: unknown) => cases.push([name, JSON.stringify(input)]);

add('structured wins', { structuredContent: { z: 1, 10: 10, 2: 2, a: [true] }, ...content(text('"ignored"')) });
for (const [name, value] of [['false', false], ['zero', 0], ['empty string', ''], ['empty array', []], ['empty object', {}]]) {
  add(`structured ${name}`, { structuredContent: value, content: false });
}
add('structured null falls through', { structuredContent: null, ...content(text('42')) });
add('structured null with empty content', { structuredContent: null, ...content() });
add('structured content bypasses a null part', { structuredContent: 7, ...content(null) });
add('JSON object split across mixed parts', content(text(' {"x":'), image, text('[1,2]} ')));
add('JSON string', content(text('"hello"')));
add('JSON boolean', content(text('false')));
add('JSON null', content(text('null')));
add('plain text joins without separators', content(text('one'), image, text('two')));
add('invalid JSON is preserved', content(text('  [x  ')));
add('invalid JSON with open brackets is preserved', content(text('not [[[[[[[[ JSON')));
add('empty content', content());
add('empty text part', content(text('')));
add('missing text part', content({ type: 'text' }));
add('null text part', content(text(null)));
add('no text parts', content(image, { type: 'TEXT', text: 'ignored' }));
add('primitive parts and arrays are ignored', content(4, false, 'text', [], text('kept')));
add('missing content', {});
add('null content', { content: null });
add('null part fails during filter', content(text('first'), null));
add('content object with a filter property', { content: { filter: [] } });
add('content string', { content: 'text' });
add('content boolean', { content: false });
add('content number on error', { isError: true, content: 1 });
add('null part on error', error(null));
add('filter completes before text coercion', content(text({ toString: 1 }), null));
add('null response', null);
add('primitive response', false);
add('array response', []);
add('non-string text is coerced', content(text(12), text(false), text({}), text([1, null, [2, 3], {}])));
add('coercion can produce JSON', content(text(true)));
add('array coercion can produce empty text', content(text([[]])));
add('coercion uses JavaScript number formatting', content(text([1e21, 1e-7, -0])));
add('non-callable toString throws', content(text({ toString: 1, valueOf: 7 })));
add('non-callable toString in nested array throws', error(text([[{ toString: null }]])));
add('tool error wins over structured content', { structuredContent: 42, ...error(text('first'), image, text('second')) });
add('tool error keeps blank lines', error(text('first'), { type: 'text' }, text(null), text('last')));
add('two empty error parts produce a newline', error(text(''), text(null)));
add('empty error text falls back', error(text('')));
add('missing error text falls back', error(image));
add('error without content falls back', { isError: true });
add('error keeps whitespace', error(text('  ')));
add('error with null content falls back', { isError: true, content: null });
add('non-string error text is coerced', error(text(false), text(0), text([null, {}])));
for (const [name, value] of [
  ['truthy string', 'false'], ['truthy number', -1], ['truthy array', []],
  ['truthy object', {}], ['falsy string', ''], ['falsy number', 0], ['falsy null', null],
]) {
  add(`isError ${name}`, { isError: value, ...content(text('5')) });
}
add('JSON text retains JS key order and duplicate semantics', content(text('{"z":0,"9":9,"2":2,"z":3,"__proto__":5}')));
add('lone surrogate in text', content(text('\ud800')));
add('lone surrogate in error', error(text('\udfff')));
add('surrogate pair joined into plain text', content(text('\ud83d'), text('\ude00')));
add('surrogate pair joined inside JSON text', content(text('"\ud83d'), text('\ude00"')));
add('escaped and raw surrogate halves form a JSON text value', content(text('"\\ud83d'), text('\ude00"')));
add('raw and escaped surrogate halves form a JSON text key', content(text('{"\ud83d'), text('\\ude00":1}')));
add('prototype-shaped values do not expose inherited MCP members',
    JSON.parse('{"__proto__":{"isError":true,"structuredContent":5}}'));
add('JSON text with a leading BOM remains plain text', content(text('\ufeff42')));
add('negative zero is preserved in JSON text', content(text('-0')));
cases.push(['structured negative zero is preserved', '{"structuredContent":-0}']);
add('overflow in JSON text is a number', content(text('1e400')));
add('nested overflow in JSON text is a number', content(text('{"big":-1e400,"small":5e-324}')));
cases.push(['overflowing number coerces to Infinity', '{"content":[{"type":"text","text":1e400}]}']);
cases.push(['overflowing isError is truthy', '{"isError":1e400}']);
cases.push(['structured overflow is a number', '{"structuredContent":1e400}']);

// TypeError wording varies by engine. Preserve the class and the operation
// that threw by deriving a stable reason from Bun's diagnostic.
const typeErrorReasons: [RegExp, string][] = [
  [/evaluating 'result.isError'/, 'cannot read isError from null'],
  [/evaluating 'c.type'/, "cannot read a text part's type from null"],
  [/filter is not a function/, 'content.filter is not a function'],
  [/^No default value$/, 'cannot convert a text part to a primitive'],
];

const rows = cases.map(([name, input]) => {
  let numberKind: string | undefined;
  let outcome: { value: unknown } | { error: string } | { thrown: string, reason: string };
  try {
    const value = extractToolResult(JSON.parse(input));
    outcome = { value };
    if (typeof value === 'number' && (!Number.isFinite(value) || Object.is(value, -0))) {
      numberKind = Object.is(value, -0) ? '-0' : String(value);
    }
  } catch (error) {
    if (error instanceof McpToolError) {
      outcome = { error: error.toolErrorText };
    } else if (error instanceof TypeError) {
      const reason = typeErrorReasons.find(([pattern]) => pattern.test(error.message))?.[1];
      if (reason === undefined) throw error;
      outcome = { thrown: error.name, reason };
    } else {
      throw error;
    }
  }
  return JSON.stringify({ name, input, outcome, numberKind });
});
writeFileSync('../conformance/golden/mcp-results.jsonl', rows.join('\n') + '\n');

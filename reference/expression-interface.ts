// Ticket 31's expression seam, using the pinned runtime directly. No runtime
// is transcribed here. Regenerate: bun reference/expression-interface.ts >
// crates/openui-runtime/tests/expressions.json
import { parseExpression } from "./lang-core/src/parser/expressions";
import { tokenize } from "./lang-core/src/parser/lexer";
import { evaluate } from "./lang-core/src/runtime/evaluator";

const state = { $hp: 5, $value: 7, $fallback: 9, $nil: null, $list: [{ n: 2 }, { n: 8 }] };
const references = { title: "HP: ", rows: [3, 6], query: { total: 12 }, x: undefined, nil: null };
(references as any).x = evaluate(parseExpression(tokenize("{a:1}.b")), {
  getState: name => (state as any)[name],
  resolveRef: name => (references as any)[name],
});
const extra = { $value: 10, $hp: null };
const expressions = [
  'title + $hp', '$value * 2', '$hp + 1', 'missing', '$missing',
  '$hp > 3 ? query.total : 0', '@Sum(rows)', '@Count($list)',
  '$list.n', '$list[1].n', '"x" + null', '5 == "5"', '1 / 0',
  '@Each($list, "row", row.n + $hp)',
  '@Each([1, 2], "row", @Each([3], "other", row + other))',
  '@Each([1, 2], "row", row)', '@Each(null, "row", row)',
  '@Each([1], "", 1)', '@Each([1])',
  '{a: query.total, b: $missing, c: {n: 1}.missing}',
  '[{n: 1}.missing, null]', '{n: 1}.missing', 'null',
  '$list == $list', 'rows == rows', '[] == []',
  '@Abs(-4)', '{k: x}', '[x]', 'x == null', '{s: $missing}',
  '[$missing]', '$fallback', '{n: nil, s: $nil}', '{k: $nil}',
  '@Abs([[1, -2], [-3]])',
];
const cases: any[] = expressions.map(expression => {
  const result = evaluate(parseExpression(tokenize(expression)), {
    getState: name => (state as any)[name],
    resolveRef: name => (references as any)[name],
    extraScope: extra,
  });
  return { expression, state, references, extra, defined: result !== undefined,
    value: result === undefined ? null : result };
});

// Resolver closures belong to the root context; Each delegates to them.
const statements = { r: "i * 10", y: "query" };
for (const expression of [
  '@Each([1,2,3],"i",r)',
  '[@Each([1,2],"i",r),@Each([5,6],"i",r)]',
  '[@Each([1,2],"i",r),r]',
  '[r,@Each([1,2],"i",r)]',
  '@Each([1,2],"query",y)',
  '@Each([1,2],"r",r)',
]) {
  const references = { i: 7, query: 17 };
  const context = {
    getState: name => state[name],
    resolveRef: name => statements[name]
      ? evaluate(parseExpression(tokenize(statements[name])), context)
      : references[name],
    extraScope: extra,
  };
  const result = evaluate(parseExpression(tokenize(expression)), context);
  cases.push({ expression, state, references, extra, statements,
    defined: result !== undefined, value: result === undefined ? null : result });
}
// A RuntimeRef inlined by parsing is already part of the Each template.
const expression = '@Each([1,2],"query",query)';
const node = parseExpression(tokenize(expression));
(node as any).args[2] = { k: "RuntimeRef", n: "query", refType: "query" };
const result = evaluate(node, { getState: () => undefined, resolveRef: () => 17 });
cases.push({ expression, state: {}, references: {query:17}, extra: {},
  inlineRuntimeRef: "query", defined: result !== undefined, value: result });
console.log(JSON.stringify(cases, null, 2));

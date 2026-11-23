// The inputs intelligent-ui-functions.ts runs through upstream's Intelligent
// UI functions: one list of argument lists per function, in upstream's group
// order. A function's ticket adds its section; each argument list is one call,
// positional as the model writes it. Values may be anything JavaScript holds:
// -0, Infinity, NaN and lone surrogates are written by the recording in its
// tagged encoding (see intelligent-ui-functions.ts).

/** One call: its positional arguments. Fewer than the params leaves the rest missing. */
export type Call = unknown[];

const NaN_ = Number.NaN;

// ─── Argument rules ──────────────────────────────────────────────────────────
// What lang-core's runtime does before a function body runs (spec #128,
// research part 3, section 0), recorded on the functions that show each rule
// and written with the rule's name in `rule`. A rule's pairs replay once their
// functions are implemented; until then the replay test counts them as
// recorded, not yet replayed.
export const argumentRules: Record<string, [string, Call][]> = {
  // The param's default applies (Range's step is 1; Default's value is optional).
  "optional null": [["Range", [0, 3, null]], ["Range", [0, 3]], ["Default", [null, 1]]],
  // The error is reported; null items aren't checked and reach the body.
  "pruned array items": [["Stats", [[1, "a", null, 2]]]],
  // The error is reported and the default applies.
  "invalid optional word": [["Format", [5, "bogus"]], ["Duration", [4800, "bogus"]]],
  // z.any() params are required, so null or nothing misses one.
  "required any": [["Append", [[1], null]], ["Default", [5, null]], ["Default", [5]]],
};

// ─── Colours (colors.ts) ─────────────────────────────────────────────────────
const mixColor: Call[] = [
  // The description's example, and the ends.
  ["#ffffff", "#0285ff", 0.5],
  ["#000000", "#ffffff", 0],
  ["#000000", "#ffffff", 1],
  ["#000000", "#ffffff", 0.25],
  ["#000000", "#ffffff", 0.75],
  ["#123456", "#fedcba", 0.3],
  // Three digits, no "#", either case.
  ["#fff", "#000", 0.5],
  ["fff", "000", 0.5],
  ["#FFF", "#abc", 0.3],
  ["#AbCdEf", "#123456", 0.1],
  ["abcdef", "#ABC", 0.9],
  // t is clamped to [0, 1].
  ["#102030", "#405060", -1],
  ["#102030", "#405060", 2],
  ["#102030", "#405060", -0],
  ["#102030", "#405060", 1e-300],
  ["#102030", "#405060", 1e308],
  ["#000000", "#ffffff", 0.9999999999999999],
  // Math.round: ties go up, and the largest double below 0.5 goes down.
  ["#000000", "#030303", 0.5],
  ["#030303", "#000000", 0.5],
  ["#000000", "#010101", 0.5],
  ["#010101", "#000000", 0.5],
  ["#000000", "#010101", 0.49999999999999994],
  ["#000000", "#ff7f01", 0.5],
  // Trimmed with JavaScript's whitespace, and only that.
  ["  #fff  ", "#000", 0.5],
  ["\t#fff\n", "\r#000\v", 0.5],
  ["\f #fff　", "﻿#000 ", 0.5],
  [" #fff ", "  #000  ", 0.5],
  ["\u0085#fff", "#000", 0.5],
  ["᠎#fff", "#000", 0.5],
  ["​#fff", "#000", 0.5],
  // Not a hex colour: the error names the argument as written.
  ["", "#000", 0.5],
  ["#", "#000", 0.5],
  ["#ff", "#000", 0.5],
  ["#ffff", "#000", 0.5],
  ["#fffff", "#000", 0.5],
  ["#fffffff", "#000", 0.5],
  ["#ggg", "#000", 0.5],
  ["##fff", "#000", 0.5],
  ["# fff", "#000", 0.5],
  ["red", "#000", 0.5],
  ["rgb(0, 0, 0)", "#000", 0.5],
  ["  nope ", "#000", 0.5],
  ["#fff\n#000", "#000", 0.5],
  ["#１２３", "#000", 0.5],
  ["#ＡＢＣ", "#000", 0.5],
  ["#٠١٢", "#000", 0.5],
  ["\u{1F600}", "#000", 0.5],
  ["é", "#000", 0.5],
  ["#fff", "nope", 0.5],
  ["bad", "worse", 0.5],
  // Lone surrogates: the error carries one back.
  ["\ud800", "#000", 0.5],
  // t that JSON can't carry.
  ["#102030", "#405060", Infinity],
  ["#102030", "#405060", -Infinity],
  ["#102030", "#405060", NaN_],
  // Argument rules: missing or null required arguments, wrong types, extras.
  [],
  ["#fff"],
  ["#fff", "#000"],
  [null, "#000", 0.5],
  ["#fff", null, 0.5],
  ["#fff", "#000", null],
  [null, null, null],
  [1, "#000", 0.5],
  ["#fff", 2, 0.5],
  ["#fff", "#000", "0.5"],
  ["#fff", "#000", true],
  [["#fff"], "#000", 0.5],
  [{ hex: "#fff" }, "#000", 0.5],
  ["#fff", "#000", 0.5, 7],
];

// lists.ts and helpers.ts. Shared inputs exercise the different callers of
// same(), including mixed number/string ids inside arrays and objects.
const equals: [unknown, unknown][] = [
  [1.0000000000000002, "1.0000000000000002"], [1e100, "1e+100"],
  [1, "1"], [1.5, "1.50"], [1e21, "1e+21"], [1e-7, "1e-7"],
  [1e-6, "0.000001"], [1e20, "100000000000000000000"], [-0, "-0"],
  [true, "true"], [null, "null"], ["é", "e\u0301"],
  [[1, { a: 2 }], ["1", { a: "2" }]], [[1], { a: 1 }],
  [{ b: 2, a: 1 }, { a: "1", b: 2 }], [{ a: 1 }, { b: 1 }],
];
const rows = [{ id: 1, data: [{ name: "Ada" }] }, { id: "1" }, null, [2]];
const positions = [-1e300, -5, -3, -1.9, -0.9, -0, 1.9, 3, 5, 1e300];
const simpleLists = [[], [1], [null, 1, true, "a", [1, 2], { id: 3 }]];
const nearLimit = Array(9999).fill(0);
const full = [...nearLimit, 1];

const lists: Record<string, Call[]> = {
  Append: [...simpleLists.map(xs => [xs, [1]]), [nearLimit, 1], [full, 1]],
  Prepend: [...simpleLists.map(xs => [xs, { id: 1 }]), [nearLimit, 1], [full, 1]],
  Concat: [[[], []], [[1], []], [[], [2]], [[1, null], [[2], 3]], [nearLimit, [1]], [nearLimit, [1, 2]]],
  Remove: [...equals.map(([a, b]) => [[a, 8, b], "", b]), [rows, "id", 1], [rows, "data.0.name", "Ada"]],
  RemoveAt: positions.map(i => [[1, 2, 3], i]),
  Update: [...equals.map(([a, b]) => [[{ id: a, kept: true }], "id", b, "id", false]),
    [rows, "id", 1, "a.b", [0]], [[{ z: 1 }], "z", 1, "0", 2],
    [[{ z: 1 }], "z", 1, "4294967295", 2], [[{ z: 1 }], "z", 1, "01", 2], [rows, "data.0.name", "Ada", "id", 2], [rows, "no", 1, "yes", 2]],
  Toggle: [...equals.map(([a, b]) => [[a, 8, a], b]), [[], 1], [full, 2]],
  Includes: [...equals.map(([a, b]) => [[a], b]), ["a12b", 12], ["x,y", ["x", "y"]],
    ["[object Object]", {}], ["1,,2", [1, null, [2]]], ["é", "e\u0301"], ["e\u0301", "e"],
    ["", ""], ["true", true], ["\uFFFD", "\ud800"], ["\uFFFD", "\uFFFD"], ["42", { toString: 42 }], [17, 1],
    ...[true, 2, null, "a", [], {}].map(some => [{ some }, 2])],
  Find: [...equals.map(([a, b]) => [[{ id: a }, { id: b }], "id", b]),
    [rows, "data.0.name", "Ada"], [rows, "data.length", 1],
    [[{}, []], "__proto__", {}], [[[], {}], "__proto__", []], [[[], {}], "__proto__.__proto__", {}],
    [[{ x: { y: 3 } }], "x.y", 3], [[{ x: ["x", "y"] }], "x.01", "y"], [rows, "absent", 0], [[], "", 1]],
  IndexOf: [...equals.map(([a, b]) => [[0, a, b], b]), [[], 1]],
  Take: positions.map(n => [[1, 2, 3], n]),
  Slice: [...positions.map(n => [[1, 2, 3], n]), ...positions.map(n => ["e\u0301😀z", 0, n]),
    ["😀", 0, 1], ["😀", 1, 2], ["ab😀cd", -4, -2], ["abc", 2, 1], [[1, 2], 1, null], [17, 0]],
  Unique: [...equals.map(([a, b]) => [[a, b, a]]), [[true, "true", null, "null", 0, -0, "0"]],
    [[[1], { a: 1 }, { a: "1" }, ["1"]]]],
  Reverse: [...simpleLists.map(xs => [xs]), [[1, "a", null, 2]], [[Infinity]], [[null]]],
  Range: [[0, 0], [0, 4], [3, -3, -2], [3, 0], [0, 3, -1], [0, 3, 0], [0, 3, -0],
    [0, 1, 0.1], [1.23456789012345, 1.234567890124, 1e-13],
    [123456789012.5, 123456789013, 0.5], [-123456789012.5, -123456789013, -0.5],
    [0, 4e-323, 5e-324], [0, 0.9, 0.3], [0, 10000], [0, 10001], [-1e308, 1e308, 1e308], [0, 1, 1e-300],
    [Infinity, 1], [-Infinity, 1], [NaN_, 1]],
  Pick: [...simpleLists, [1, 2], [1, 2, 3, 4, 5]].map(xs => [xs]),
  Shuffle: [...simpleLists, [1, 2], [1, 2, 3, 4, 5], [1, 1, 1]].map(xs => [xs]),
  Flatten: [...simpleLists.map(xs => [xs]), [[[1, [2]], [], 3]], [[nearLimit, [1]]], [[full, [1]]]],
  Chunk: [...positions.map(n => [[1, 2, 3, 4, 5], n]), [[], 2], [[1, 2, 3], 2], [[1], 0.99]],
};
// Missing, null and wrong-type parameters reach the same validated calls as
// the model's programs do.
for (const calls of Object.values(lists)) calls.push([], [null], [false]);

/** Every function's calls, by name. */
export const cases: Record<string, Call[]> = {
  ...lists,
  MixColor: mixColor,
};
